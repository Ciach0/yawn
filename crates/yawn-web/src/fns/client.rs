use super::batch::{self, BATCH_CONTENT_TYPE, BATCH_URL, Call, Queue, Reply};
use super::codec::{Codec, DecodeError};
use super::{CALL_HEADER, Error, Method, OUTDATED, encode_args, url};
use crate::dom::{FetchError, FetchRequest, fetch_bytes, spawn_local};
use std::cell::RefCell;

type Answer = Result<Reply, Error>;

thread_local! {
    static QUEUE: RefCell<Queue<Answer>> = RefCell::new(Queue::default());
}

#[doc(hidden)]
pub async fn call<C, A, T>(method: Method, module: &str, name: &str, args: &A) -> Result<T, Error>
where
    C: Codec<A> + Codec<T>,
{
    let args = <C as Codec<A>>::encode(args).map_err(Error::Body)?;
    let content_type = <C as Codec<A>>::CONTENT_TYPE;
    let reply = send(Call { method, url: url(module, name), args, content_type }).await?;
    if reply.status == OUTDATED {
        return Err(Error::Outdated);
    }
    if !(200..300).contains(&reply.status) {
        return Err(Error::Status(reply.status, String::from_utf8_lossy(&reply.body).into_owned()));
    }
    <C as Codec<T>>::decode(&reply.body).map_err(|e| match e {
        DecodeError::Schema => Error::Outdated,
        DecodeError::Malformed(m) => Error::Body(m),
    })
}

/// Puts the call in the queue; the first call of a tick starts the flush, which sends what has
/// gathered by then
async fn send(call: Call) -> Answer {
    let (slot, first) = QUEUE.with(|q| q.borrow_mut().push(call));
    if first {
        spawn_local(flush());
    }
    slot.await
}

async fn flush() {
    let groups = QUEUE.with(|q| q.borrow_mut().drain());
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
