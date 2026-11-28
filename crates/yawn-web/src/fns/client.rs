use super::batch::{self, BATCH_CONTENT_TYPE, BATCH_URL, Call, Queue, Reply, Slot};
use super::codec::{self, Codec, DecodeError};
use super::pipe::{ArgBuilder, Pending};
use super::{CALL_HEADER, Error, Method, OUTDATED, encode_args, url};
use crate::dom::{FetchError, FetchRequest, fetch_bytes, spawn_local};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

type Answer = Result<Reply, Error>;

thread_local! {
    static QUEUE: RefCell<Queue<Answer>> = RefCell::new(Queue::default());
    static EPOCH: Cell<u32> = const { Cell::new(0) };
}

/// Which tick the queue is gathering: a piece of an answer is good for the one it was made in
pub(super) fn epoch() -> u32 {
    EPOCH.with(Cell::get)
}

/// A call in the queue whose answer is a `T`
pub struct Waiting<T> {
    slot: Slot<Answer>,
    position: u32,
    epoch: u32,
    failed: Option<Error>,
    marker: PhantomData<fn() -> T>,
}

impl<T> Waiting<T> {
    pub(super) fn position(&self) -> u32 {
        self.position
    }

    pub(super) fn epoch(&self) -> u32 {
        self.epoch
    }
}

impl<T: yawn_bits::Bits> Future for Waiting<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(e) = self.failed.take() {
            return Poll::Ready(Err(e));
        }
        match Pin::new(&mut self.slot).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(answer) => Poll::Ready(answer.and_then(|r| open::<codec::Bits, T>(&r))),
        }
    }
}

/// A call that takes pieces of the answers of earlier calls of this tick, queued with them
#[doc(hidden)]
pub fn pipe<T: yawn_bits::Bits>(
    method: Method,
    module: &str,
    name: &str,
    args: ArgBuilder,
) -> Pending<T> {
    let epoch = epoch();
    if let Some(error) = args.error {
        let (_, slot) = batch::slot();
        return Pending {
            inner: Waiting { slot, position: 0, epoch, failed: Some(error), marker: PhantomData },
        };
    }
    let call = Call {
        method,
        url: url(module, name),
        args: args.bytes,
        holes: args.holes,
        content_type: <codec::Bits as Codec<()>>::CONTENT_TYPE,
    };
    let (slot, first, position) = QUEUE.with(|q| q.borrow_mut().push(call));
    if first {
        spawn_local(flush());
    }
    Pending { inner: Waiting { slot, position, epoch, failed: None, marker: PhantomData } }
}

fn open<C: Codec<T>, T>(reply: &Reply) -> Result<T, Error> {
    if reply.status == OUTDATED {
        return Err(Error::Outdated);
    }
    if !(200..300).contains(&reply.status) {
        return Err(Error::Status(reply.status, String::from_utf8_lossy(&reply.body).into_owned()));
    }
    C::decode(&reply.body).map_err(|e| match e {
        DecodeError::Schema => Error::Outdated,
        DecodeError::Malformed(m) => Error::Body(m),
    })
}

#[doc(hidden)]
pub async fn call<C, A, T>(method: Method, module: &str, name: &str, args: &A) -> Result<T, Error>
where
    C: Codec<A> + Codec<T>,
{
    let args = <C as Codec<A>>::encode(args).map_err(Error::Body)?;
    let content_type = <C as Codec<A>>::CONTENT_TYPE;
    let reply =
        send(Call { method, url: url(module, name), args, holes: Vec::new(), content_type })
            .await?;
    open::<C, T>(&reply)
}

/// Puts the call in the queue; the first call of a tick starts the flush, which sends what has
/// gathered by then
async fn send(call: Call) -> Answer {
    let (slot, first, _) = QUEUE.with(|q| q.borrow_mut().push(call));
    if first {
        spawn_local(flush());
    }
    slot.await
}

async fn flush() {
    let groups = QUEUE.with(|q| q.borrow_mut().drain());
    EPOCH.with(|e| e.set(e.get().wrapping_add(1)));
    for group in groups {
        spawn_local(async move {
            let (calls, fillers): (Vec<_>, Vec<_>) = group.into_iter().unzip();
            let answers =
                if calls.len() == 1 { vec![single(&calls[0]).await] } else { many(&calls).await };
            for (filler, answer) in fillers.into_iter().zip(answers) {
                filler.fill(answer);
            }
        });
    }
}

fn transport(e: FetchError) -> Error {
    match e {
        FetchError::Timeout => Error::Network("timed out".into()),
        FetchError::Header(m) | FetchError::Network(m) => Error::Network(m),
    }
}

async fn single(call: &Call) -> Answer {
    let (url, headers, body) = if call.method == Method::Get {
        let url = format!("{}?args={}", call.url, encode_args(&call.args));
        (url, [(CALL_HEADER.to_string(), "1".to_string())], Vec::new())
    } else {
        let headers = [("content-type".to_string(), call.content_type.to_string())];
        (call.url.clone(), headers, call.args.clone())
    };
    let r = fetch_bytes(FetchRequest {
        url: &url,
        method: call.method.as_str(),
        headers: &headers,
        body: &body,
        timeout_ms: 30_000,
    })
    .await
    .map_err(transport)?;
    Ok(Reply { status: r.status, body: r.body })
}

async fn many(calls: &[Call]) -> Vec<Answer> {
    let body = batch::encode_request(calls);
    #[cfg(feature = "socket")]
    if let Some(answers) = over_socket(&body, calls.len()).await {
        return answers;
    }
    let headers = [("content-type".to_string(), BATCH_CONTENT_TYPE.to_string())];
    let fetched = fetch_bytes(FetchRequest {
        url: BATCH_URL,
        method: "POST",
        headers: &headers,
        body: &body,
        timeout_ms: 30_000,
    })
    .await;
    let failed = |e: Error| calls.iter().map(|_| Err(e.clone())).collect::<Vec<_>>();
    let r = match fetched {
        Ok(r) => r,
        Err(e) => return failed(transport(e)),
    };
    if !(200..300).contains(&r.status) {
        return failed(Error::Status(r.status, String::from_utf8_lossy(&r.body).into_owned()));
    }
    match batch::decode_response(&r.body, calls.len()) {
        Some(replies) => replies.into_iter().map(Ok).collect(),
        None => failed(Error::Body("the batch answer is not readable".into())),
    }
}

/// The batch over the socket. A batch that did not get through (no socket, closed on the way) goes
/// again by fetch, so the socket is only ever a faster road; a batch the server refused to read is
/// an answer, not a reason to try again.
#[cfg(feature = "socket")]
async fn over_socket(body: &[u8], calls: usize) -> Option<Vec<Answer>> {
    let frame = crate::dom::socket_batch(body).await?;
    let (&ok, rest) = frame.split_first()?;
    if ok != 0 {
        let e = Error::Body("the server could not read the batch".into());
        return Some((0..calls).map(|_| Err(e.clone())).collect());
    }
    match batch::decode_response(rest, calls) {
        Some(replies) => Some(replies.into_iter().map(Ok).collect()),
        None => {
            let e = Error::Body("the batch answer is not readable".into());
            Some((0..calls).map(|_| Err(e.clone())).collect())
        }
    }
}
