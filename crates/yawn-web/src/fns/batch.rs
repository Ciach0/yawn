//! Several calls in one request. The client collects the calls made in the same tick and sends
//! them to `/__yawn/batch`; the server runs them in order (the reads between two writes at once)
//! and answers each. The framing is the part both sides share.

use super::Method;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use yawn_bits::{Reader, put_varint};

pub const BATCH_URL: &str = "/__yawn/batch";
pub const BATCH_CONTENT_TYPE: &str = "application/vnd.yawn.batch";

/// The most calls in one batch; the client splits a larger one
pub const MAX_CALLS: usize = 32;

/// What the server accepts: more than the client sends, to leave room for another client
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub(crate) const MAX_CALLS_SERVED: usize = 64;

/// One call: the method of the function, its URL and its arguments as its codec wrote them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub method: Method,
    pub url: String,
    pub args: Vec<u8>,
    /// What a lone call is sent with. Not part of a batch: the server knows it from the function.
    pub content_type: &'static str,
}

/// One answer: the status the call alone would have had, and its body
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

fn method_byte(m: Method) -> u8 {
    match m {
        Method::Get => 0,
        Method::Post => 1,
        Method::Put => 2,
        Method::Patch => 3,
        Method::Delete => 4,
    }
}

fn method_from(b: u8) -> Option<Method> {
    Some(match b {
        0 => Method::Get,
        1 => Method::Post,
        2 => Method::Put,
        3 => Method::Patch,
        4 => Method::Delete,
        _ => return None,
    })
}

fn put_bytes(w: &mut Vec<u8>, bytes: &[u8]) {
    put_varint(w, bytes.len() as u64);
    w.extend_from_slice(bytes);
}

fn get_bytes<'a>(r: &mut Reader<'a>) -> &'a [u8] {
    let n = r.len(1);
    r.take(n)
}

pub fn encode_request(calls: &[Call]) -> Vec<u8> {
    let mut w = Vec::new();
    put_varint(&mut w, calls.len() as u64);
    for c in calls {
        w.push(method_byte(c.method));
        put_bytes(&mut w, c.url.as_bytes());
        put_bytes(&mut w, &c.args);
    }
    w
}

pub fn decode_request(bytes: &[u8], limit: usize) -> Option<Vec<Call>> {
    let mut r = Reader::new(bytes);
    let n = r.len(3);
    if n > limit {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let method = method_from(r.byte());
        let url = String::from_utf8(get_bytes(&mut r).to_vec()).ok();
        let args = get_bytes(&mut r).to_vec();
        if r.failed() {
            return None;
        }
        out.push(Call { method: method?, url: url?, args, content_type: "" });
    }
    yawn_bits::finish(r).ok()?;
    Some(out)
}

pub fn encode_response(replies: &[Reply]) -> Vec<u8> {
    let mut w = Vec::new();
    put_varint(&mut w, replies.len() as u64);
    for r in replies {
        put_varint(&mut w, r.status as u64);
        put_bytes(&mut w, &r.body);
    }
    w
}

pub fn decode_response(bytes: &[u8], expect: usize) -> Option<Vec<Reply>> {
    let mut r = Reader::new(bytes);
    let n = r.len(2);
    if n != expect {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let status = u16::try_from(r.varint()).ok();
        let body = get_bytes(&mut r).to_vec();
        if r.failed() {
            return None;
        }
        out.push(Reply { status: status?, body });
    }
    yawn_bits::finish(r).ok()?;
    Some(out)
}

/// The receiving end of one call in a batch: a future the sender fills in
pub struct Slot<T>(Rc<RefCell<SlotState<T>>>);

struct SlotState<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

/// The sending end of a [`Slot`]
pub struct Filler<T>(Rc<RefCell<SlotState<T>>>);

pub fn slot<T>() -> (Filler<T>, Slot<T>) {
    let state = Rc::new(RefCell::new(SlotState { value: None, waker: None }));
    (Filler(state.clone()), Slot(state))
}

impl<T> Filler<T> {
    pub fn fill(self, value: T) {
        let waker = {
            let mut s = self.0.borrow_mut();
            s.value = Some(value);
            s.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

impl<T> Future for Slot<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut s = self.0.borrow_mut();
        match s.value.take() {
            Some(v) => Poll::Ready(v),
            None => {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// The calls waiting for the end of the tick, with where each answer goes
pub struct Queue<T> {
    waiting: Vec<(Call, Filler<T>)>,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Queue { waiting: Vec::new() }
    }
}

impl<T> Queue<T> {
    /// Adds a call; true if it is the first of this tick, so the caller schedules the flush
    pub fn push(&mut self, call: Call) -> (Slot<T>, bool) {
        let first = self.waiting.is_empty();
        let (filler, slot) = slot();
        self.waiting.push((call, filler));
        (slot, first)
    }

    /// Everything waiting, in groups of at most [`MAX_CALLS`]
    pub fn drain(&mut self) -> Vec<Vec<(Call, Filler<T>)>> {
        let mut all = std::mem::take(&mut self.waiting);
        let mut groups = Vec::new();
        while all.len() > MAX_CALLS {
            let rest = all.split_off(MAX_CALLS);
            groups.push(std::mem::replace(&mut all, rest));
        }
        if !all.is_empty() {
            groups.push(all);
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{RawWaker, RawWakerVTable};

    fn call(n: u8) -> Call {
        Call {
            method: Method::Post,
            url: format!("/__yawn/fn/f{n}"),
            args: vec![n; n as usize],
            content_type: "",
        }
    }

    #[test]
    fn requests_and_responses_round_trip() {
        let calls = vec![
            call(0),
            call(3),
            Call { method: Method::Get, url: "/a".into(), args: vec![], content_type: "" },
        ];
        assert_eq!(decode_request(&encode_request(&calls), 8), Some(calls));
        let replies =
            vec![Reply { status: 200, body: vec![1, 2] }, Reply { status: 409, body: vec![] }];
        assert_eq!(decode_response(&encode_response(&replies), 2), Some(replies));
        assert_eq!(decode_response(&encode_response(&[]), 1), None);
        assert_eq!(decode_request(&encode_request(&[call(1), call(2)]), 1), None);
    }

    #[test]
    fn corrupt_framing_is_refused() {
        let good = encode_request(&[call(1), call(2), call(3)]);
        for cut in 0..good.len() {
            assert_eq!(decode_request(&good[..cut], 8), None, "cut at {cut}");
        }
        let mut bad_method = good.clone();
        bad_method[1] = 9;
        assert_eq!(decode_request(&bad_method, 8), None);
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(decode_request(&trailing, 8), None);
        let mut bomb = Vec::new();
        put_varint(&mut bomb, 1 << 40);
        assert_eq!(decode_request(&bomb, 8), None);
        let mut seed = 7u64;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut b = good.clone();
            let at = (seed as usize) % b.len();
            b[at] = (seed >> 8) as u8;
            let _ = decode_request(&b, 8);
            let _ = decode_response(&b, 3);
        }
    }

    fn noop_waker() -> Waker {
        const VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    #[test]
    fn a_slot_resolves_when_filled() {
        let (filler, mut slot) = slot::<u8>();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        assert_eq!(Pin::new(&mut slot).poll(&mut cx), Poll::Pending);
        filler.fill(7);
        assert_eq!(Pin::new(&mut slot).poll(&mut cx), Poll::Ready(7));
    }

    #[test]
    fn a_queue_groups_the_tick() {
        let mut q = Queue::<u8>::default();
        let (_, first) = q.push(call(1));
        let (_, second) = q.push(call(2));
        assert!(first && !second);
        assert_eq!(q.drain().len(), 1);
        let (_, again) = q.push(call(3));
        assert!(again);
        for n in 0..(MAX_CALLS + 5) {
            q.push(call(n as u8));
        }
        let groups = q.drain();
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), vec![MAX_CALLS, 6]);
        assert!(q.drain().is_empty());
    }
}
