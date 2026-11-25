use super::{Error, RequestBuilder, Response};
use crate::dom::{self, FetchError, FetchRequest};

pub(super) async fn send(req: RequestBuilder) -> Result<Response, Error> {
    let sent = dom::fetch_bytes(FetchRequest {
        url: &req.url,
        method: req.method.as_str(),
        headers: &req.headers,
        body: &req.body,
        timeout_ms: req.timeout.as_millis().min(i32::MAX as u128) as i32,
    })
    .await;
    match sent {
        Ok(r) => Ok(Response { status: r.status, headers: r.headers, body: r.body }),
        Err(FetchError::Timeout) => Err(Error::Timeout),
        Err(FetchError::Header(e)) => Err(Error::Url(e)),
        Err(FetchError::Network(e)) => Err(Error::Network(e)),
    }
}
