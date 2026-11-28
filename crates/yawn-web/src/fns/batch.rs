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

/// The most references to earlier answers one call carries, and the deepest path in one
pub const MAX_HOLES: usize = 8;
pub const MAX_PATH: usize = 8;

/// A place in a call's arguments that an earlier call of the batch fills in: the bytes of the
/// field `path` of its answer go in at `at`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hole {
    /// Where in `args`, counted in bytes
    pub at: usize,
    /// The earlier call of the batch, by position
    pub from: u32,
    pub path: Vec<u32>,
    /// The hash of the field's type the caller expects, as a message header folds it
    pub schema: u32,
}

/// One call: the method of the function, its URL and its arguments as its codec wrote them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub method: Method,
    pub url: String,
    pub args: Vec<u8>,
    pub holes: Vec<Hole>,
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
        put_varint(&mut w, c.holes.len() as u64);
        for h in &c.holes {
            put_varint(&mut w, h.at as u64);
            put_varint(&mut w, h.from as u64);
            put_varint(&mut w, h.schema as u64);
            put_varint(&mut w, h.path.len() as u64);
            for &p in &h.path {
                put_varint(&mut w, p as u64);
            }
        }
    }
    w
}

pub fn decode_request(bytes: &[u8], limit: usize) -> Option<Vec<Call>> {
    let mut r = Reader::new(bytes);
    let n = r.len(4);
    if n > limit {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let method = method_from(r.byte());
        let url = String::from_utf8(get_bytes(&mut r).to_vec()).ok();
        let args = get_bytes(&mut r).to_vec();
        let holes_n = r.len(1);
        if holes_n > MAX_HOLES {
            return None;
        }
        let mut holes = Vec::with_capacity(holes_n);
        for _ in 0..holes_n {
            let at = usize::try_from(r.varint()).ok()?;
            let from = u32::try_from(r.varint()).ok()?;
            let schema = u32::try_from(r.varint()).ok()?;
            let path_n = r.len(1);
            if path_n > MAX_PATH {
                return None;
            }
            let path: Vec<u32> = (0..path_n).map(|_| r.varint() as u32).collect();
            let after = holes.last().map_or(0, |h: &Hole| h.at);
            if r.failed() || at > args.len() || at < after || from as usize >= i {
                return None;
            }
            holes.push(Hole { at, from, path, schema });
        }
        if r.failed() {
            return None;
        }
        out.push(Call { method: method?, url: url?, args, holes, content_type: "" });
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
    /// Adds a call; true if it is the first of this tick, so the caller schedules the flush. The
    /// position is what a later call's [`Hole`] names.
    pub fn push(&mut self, call: Call) -> (Slot<T>, bool, u32) {
        let first = self.waiting.is_empty();
        let position = self.waiting.len() as u32;
        let (filler, slot) = slot();
        self.waiting.push((call, filler));
        (slot, first, position)
    }

    /// Everything waiting, in groups of at most [`MAX_CALLS`]; a call that names an earlier one
    /// stays in its group where it can, and the positions in the holes are made relative to the
    /// group. A hole that names a call of another group cannot be filled and is left pointing past
    /// the batch, which the server refuses.
    pub fn drain(&mut self) -> Vec<Vec<(Call, Filler<T>)>> {
        let all = std::mem::take(&mut self.waiting);
        let n = all.len();
        let mut cut = vec![true; n + 1];
        for (i, (call, _)) in all.iter().enumerate() {
            for h in &call.holes {
                for c in cut.iter_mut().take(i + 1).skip(h.from as usize + 1) {
                    *c = false;
                }
            }
        }
        let mut groups: Vec<Vec<(Call, Filler<T>)>> = Vec::new();
        let mut starts = vec![0usize];
        let mut start = 0;
        while start < n {
            let limit = (start + MAX_CALLS).min(n);
            let end = (start + 1..=limit).rev().find(|&k| cut[k]).unwrap_or(limit);
            starts.push(end);
            start = end;
        }
        let mut items = all.into_iter();
        for w in starts.windows(2) {
            let base = w[0] as u32;
            let mut group = Vec::with_capacity(w[1] - w[0]);
            for _ in w[0]..w[1] {
                let (mut call, filler) = items.next().expect("every call is in a group");
                for h in &mut call.holes {
                    h.from = if h.from >= base { h.from - base } else { u32::MAX };
                }
                group.push((call, filler));
            }
            groups.push(group);
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
            holes: vec![],
            content_type: "",
        }
    }

    #[test]
    fn requests_and_responses_round_trip() {
        let calls = vec![
            call(0),
            call(3),
            Call {
                method: Method::Get,
                url: "/a".into(),
                args: vec![],
                holes: vec![],
                content_type: "",
            },
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

    fn piped(from: u32, at: usize) -> Call {
        Call {
            holes: vec![Hole { at, from, path: vec![0, 2], schema: 9 }],
            args: vec![1, 2, 3, 4],
            ..call(1)
        }
    }

    #[test]
    fn holes_round_trip_and_are_checked() {
        let calls = vec![
            call(1),
            piped(0, 2),
            Call {
                holes: vec![
                    Hole { at: 0, from: 0, path: vec![], schema: 1 },
                    Hole { at: 4, from: 1, path: vec![7], schema: 2 },
                ],
                ..piped(0, 0)
            },
        ];
        assert_eq!(decode_request(&encode_request(&calls), 8), Some(calls));
        assert_eq!(
            decode_request(&encode_request(&[piped(0, 2)]), 8),
            None,
            "names a call that is not before it"
        );
        assert_eq!(
            decode_request(&encode_request(&[call(1), piped(1, 2)]), 8),
            None,
            "names itself"
        );
        assert_eq!(
            decode_request(&encode_request(&[call(1), piped(0, 5)]), 8),
            None,
            "past the arguments"
        );
        let unordered = Call {
            holes: vec![
                Hole { at: 3, from: 0, path: vec![], schema: 1 },
                Hole { at: 1, from: 0, path: vec![], schema: 1 },
            ],
            ..piped(0, 0)
        };
        assert_eq!(decode_request(&encode_request(&[call(1), unordered]), 8), None);
        let many = Call {
            holes: (0..=MAX_HOLES)
                .map(|_| Hole { at: 0, from: 0, path: vec![], schema: 1 })
                .collect(),
            ..piped(0, 0)
        };
        assert_eq!(decode_request(&encode_request(&[call(1), many]), 8), None);
        let deep = Call {
            holes: vec![Hole { at: 0, from: 0, path: vec![0; MAX_PATH + 1], schema: 1 }],
            ..piped(0, 0)
        };
        assert_eq!(decode_request(&encode_request(&[call(1), deep]), 8), None);
    }

    #[test]
    fn a_chain_stays_in_one_group_and_its_holes_become_relative() {
        let mut q = Queue::<u8>::default();
        for n in 0..(MAX_CALLS - 2) {
            q.push(call(n as u8));
        }
        let (_, _, a) = q.push(call(200));
        let (_, _, b) = q.push(piped(a, 0));
        q.push(piped(b, 0));
        q.push(call(1));
        let groups = q.drain();
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), vec![MAX_CALLS - 2, 4]);
        assert_eq!(groups[1][1].0.holes[0].from, 0);
        assert_eq!(groups[1][2].0.holes[0].from, 1);
    }

    #[test]
    fn a_queue_groups_the_tick() {
        let mut q = Queue::<u8>::default();
        let (_, first, _) = q.push(call(1));
        let (_, second, _) = q.push(call(2));
        assert!(first && !second);
        assert_eq!(q.drain().len(), 1);
        let (_, again, _) = q.push(call(3));
        assert!(again);
        for n in 0..(MAX_CALLS + 5) {
            q.push(call(n as u8));
        }
        let groups = q.drain();
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), vec![MAX_CALLS, 6]);
        assert!(q.drain().is_empty());
    }
}
