use super::batch::{self, BATCH_CONTENT_TYPE, BATCH_URL, MAX_CALLS_SERVED, Reply};
use super::codec::{Codec, DecodeError};
use super::{CALL_HEADER, Error, Method, OUTDATED, decode_args};
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, RwLock};

/// Runs `f` as if `parts` were the request being answered
#[doc(hidden)]
pub async fn with_request<F: Future>(parts: Parts, f: F) -> F::Output {
    REQUEST.scope(Arc::new(tokio::sync::Mutex::new(parts)), f).await
}

/// The most a call may send, as axum's own body limit
const BODY_LIMIT: usize = 2 * 1024 * 1024;

tokio::task_local! {
    /// The request a server function is answering, for [`extract`]
    static REQUEST: Arc<tokio::sync::Mutex<Parts>>;
}

/// An extractor of the request that called the server function (`#[post(session: Session)]`):
/// `headers`, `Extension`s, anything axum can take from a request's parts. A call that did not
/// come over HTTP has no request, and fails.
#[doc(hidden)]
pub async fn extract<T: FromRequestParts<()>>() -> Result<T, Error> {
    let Ok(parts) = REQUEST.try_with(Arc::clone) else {
        return Err(Error::Status(500, "this call has no request to read".into()));
    };
    let mut parts = parts.lock().await;
    T::from_request_parts(&mut parts, &()).await.map_err(|rejection| {
        Error::Status(rejection.into_response().status().as_u16(), "request refused".into())
    })
}

/// A server function as the router calls it: `C` the codec of its arguments and answer, `Args`
/// the tuple of its argument types
#[doc(hidden)]
pub trait Handler<C, Args>: Clone + Send + Sync + 'static {
    /// Runs the call. `wanted` are the paths into the answer that later calls of a batch take their
    /// arguments from; the second part of the result has what each came to.
    fn run(self, body: Bytes, wanted: Vec<Vec<u32>>) -> impl Future<Output = Answered> + Send;
}

/// The answer of a call and the pieces of it that the calls after it in a batch asked for
#[doc(hidden)]
pub type Answered = (Response, Vec<Option<(Vec<u8>, u64)>>);

fn answer<C: Codec<T>, T>(out: Result<T, Error>, wanted: &[Vec<u32>]) -> Answered {
    let projected = match &out {
        Ok(v) => wanted.iter().map(|path| C::project(v, path)).collect(),
        Err(_) => wanted.iter().map(|_| None).collect(),
    };
    (render::<C, T>(out), projected)
}

fn render<C: Codec<T>, T>(out: Result<T, Error>) -> Response {
    match out {
        Ok(v) => match C::encode(&v) {
            Ok(bytes) => ([(header::CONTENT_TYPE, C::CONTENT_TYPE)], bytes).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
        Err(Error::Status(code, message)) => {
            (StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), message)
                .into_response()
        }
        Err(Error::Outdated) => (StatusCode::CONFLICT, "outdated").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn unreadable(e: DecodeError) -> Response {
    match e {
        DecodeError::Schema => {
            (StatusCode::from_u16(OUTDATED).unwrap(), "outdated").into_response()
        }
        DecodeError::Malformed(m) => {
            (StatusCode::BAD_REQUEST, format!("cannot read the arguments: {m}")).into_response()
        }
    }
}

macro_rules! handlers {
    ($($a:ident: $t:ident),*) => {
        impl<C, F, Fut, T, $($t),*> Handler<C, ($($t,)*)> for F
        where
            C: Codec<($($t,)*)> + Codec<T> + 'static,
            F: Fn($($t),*) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = Result<T, Error>> + Send,
            $($t: Send,)*
        {
            async fn run(self, body: Bytes, wanted: Vec<Vec<u32>>) -> Answered {
                let nothing = || wanted.iter().map(|_| None).collect();
                match <C as Codec<($($t,)*)>>::decode(&body) {
                    Ok(($($a,)*)) => answer::<C, T>(self($($a),*).await, &wanted),
                    Err(e) => (unreadable(e), nothing()),
                }
            }
        }
    };
}
handlers!();
handlers!(a: A);
handlers!(a: A, b: B);
handlers!(a: A, b: B, c: C2);
handlers!(a: A, b: B, c: C2, d: D);
handlers!(a: A, b: B, c: C2, d: D, e: E);
handlers!(a: A, b: B, c: C2, d: D, e: E, f: G);
handlers!(a: A, b: B, c: C2, d: D, e: E, f: G, g: H);
handlers!(a: A, b: B, c: C2, d: D, e: E, f: G, g: H, h: I);

type Run = Arc<
    dyn Fn(Bytes, Vec<Vec<u32>>) -> Pin<Box<dyn Future<Output = Answered> + Send>> + Send + Sync,
>;

struct Entry {
    method: Method,
    run: Run,
}

/// Every server function by URL, for a batch to find what its calls name
static REGISTRY: LazyLock<RwLock<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn lookup(url: &str) -> Option<(Method, Run)> {
    let map = REGISTRY.read().ok()?;
    map.get(url).map(|e| (e.method, e.run.clone()))
}

/// Answers a call. A `GET` needs the `x-yawn-fn` header and the others the content type of their
/// codec: a cross-site page can send neither, so it can't call a function with the user's cookies
async fn serve(
    method: Method,
    content_type: &str,
    run: Run,
    headers: &HeaderMap,
    query: Option<&str>,
    body: Bytes,
) -> Response {
    if method == Method::Get {
        if !headers.contains_key(CALL_HEADER) {
            return (StatusCode::FORBIDDEN, "a server function call carries x-yawn-fn")
                .into_response();
        }
        return match query.and_then(decode_args) {
            Some(args) => run(Bytes::from(args), Vec::new()).await.0,
            None => (StatusCode::BAD_REQUEST, "the call has no readable args").into_response(),
        };
    }
    if !has_content_type(headers, content_type) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("this server function takes {content_type}"),
        )
            .into_response();
    }
    run(body, Vec::new()).await.0
}

fn has_content_type(headers: &HeaderMap, want: &str) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|m| m.trim().eq_ignore_ascii_case(want))
}

/// Adds the route of one server function, and lists it for batches
#[doc(hidden)]
pub fn route<C, A, F>(router: axum::Router, url: &str, method: Method, f: F) -> axum::Router
where
    C: Codec<()> + 'static,
    A: 'static,
    F: Handler<C, A>,
    C: Send,
{
    let content_type = <C as Codec<()>>::CONTENT_TYPE;
    let run: Run = {
        let f = f.clone();
        Arc::new(move |body, wanted| {
            let f = f.clone();
            Box::pin(async move { Handler::<C, A>::run(f, body, wanted).await })
        })
    };
    if let Ok(mut map) = REGISTRY.write() {
        map.insert(url.to_string(), Entry { method, run: run.clone() });
    }
    macro_rules! verb {
        ($verb:ident) => {
            axum::routing::$verb(move |request: Request| {
                let run = run.clone();
                async move {
                    let (parts, body) = request.into_parts();
                    let Ok(body) = axum::body::to_bytes(body, BODY_LIMIT).await else {
                        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
                    };
                    let (headers, query) =
                        (parts.headers.clone(), parts.uri.query().map(str::to_owned));
                    let parts = Arc::new(tokio::sync::Mutex::new(parts));
                    REQUEST
                        .scope(
                            parts,
                            serve(method, content_type, run, &headers, query.as_deref(), body),
                        )
                        .await
                }
            })
        };
    }
    let method_router = match method {
        Method::Get => verb!(get),
        Method::Post => verb!(post),
        Method::Put => verb!(put),
        Method::Patch => verb!(patch),
        Method::Delete => verb!(delete),
    };
    router.route(url, method_router)
}

/// The route that runs the calls of a batch
#[doc(hidden)]
pub fn batch_route(router: axum::Router) -> axum::Router {
    router.route(BATCH_URL, axum::routing::post(batch))
}

/// The request one call of a batch is answered under: the batch's own, with the call's method and
/// URL, so an extractor sees the same headers and extensions
fn request_of(batch: &Parts, call: &batch::Call) -> Option<Parts> {
    let (mut parts, ()) = Request::new(()).into_parts();
    parts.method = match call.method {
        Method::Get => axum::http::Method::GET,
        Method::Post => axum::http::Method::POST,
        Method::Put => axum::http::Method::PUT,
        Method::Patch => axum::http::Method::PATCH,
        Method::Delete => axum::http::Method::DELETE,
    };
    parts.uri = call.url.parse().ok()?;
    parts.headers = batch.headers.clone();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.extensions = batch.extensions.clone();
    Some(parts)
}

async fn reply_of(response: Response) -> Reply {
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap_or_default();
    Reply { status, body: body.to_vec() }
}

fn refused(status: StatusCode, why: &str) -> Reply {
    Reply { status: status.as_u16(), body: why.as_bytes().to_vec() }
}

/// What the call at a position answered, and the pieces of it later calls took
struct Done {
    reply: Reply,
    pieces: Vec<(Vec<u32>, Option<(Vec<u8>, u64)>)>,
}

async fn finish(answered: Answered, wanted: Vec<Vec<u32>>) -> Done {
    let (response, projected) = answered;
    Done { reply: reply_of(response).await, pieces: wanted.into_iter().zip(projected).collect() }
}

/// The arguments of a call with the pieces of earlier answers put in the places it left open
fn assemble(call: &batch::Call, done: &[Option<Done>]) -> Result<Vec<u8>, Reply> {
    let mut out = Vec::with_capacity(call.args.len());
    let mut from = 0;
    for hole in &call.holes {
        let Some(source) = done.get(hole.from as usize).and_then(Option::as_ref) else {
            return Err(refused(
                StatusCode::FAILED_DEPENDENCY,
                "the call it depends on is not before it",
            ));
        };
        if !(200..300).contains(&source.reply.status) {
            return Err(refused(StatusCode::FAILED_DEPENDENCY, "a call it depends on failed"));
        }
        let piece =
            source.pieces.iter().find(|(path, _)| *path == hole.path).and_then(|(_, p)| p.as_ref());
        let Some((bytes, schema)) = piece else {
            return Err(refused(StatusCode::FAILED_DEPENDENCY, "the answer has no such field"));
        };
        if yawn_bits::folded_schema(*schema) != hole.schema {
            return Err(refused(StatusCode::CONFLICT, "outdated"));
        }
        out.extend_from_slice(&call.args[from..hole.at]);
        out.extend_from_slice(bytes);
        from = hole.at;
    }
    out.extend_from_slice(&call.args[from..]);
    if out.len() > BODY_LIMIT {
        return Err(refused(StatusCode::PAYLOAD_TOO_LARGE, "the arguments grew too large"));
    }
    Ok(out)
}

async fn batch(request: Request) -> Response {
    let (parts, body) = request.into_parts();
    if !has_content_type(&parts.headers, BATCH_CONTENT_TYPE) {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("a batch takes {BATCH_CONTENT_TYPE}"))
            .into_response();
    }
    let Ok(body) = axum::body::to_bytes(body, BODY_LIMIT).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Some(calls) = batch::decode_request(&body, MAX_CALLS_SERVED) else {
        return (StatusCode::BAD_REQUEST, "the batch is not readable").into_response();
    };
    let mut wanted: Vec<Vec<Vec<u32>>> = vec![Vec::new(); calls.len()];
    for call in &calls {
        for hole in &call.holes {
            let paths = &mut wanted[hole.from as usize];
            if !paths.contains(&hole.path) {
                paths.push(hole.path.clone());
            }
        }
    }
    let mut done: Vec<Option<Done>> = (0..calls.len()).map(|_| None).collect();
    let mut reads: Vec<(usize, tokio::task::JoinHandle<Done>)> = Vec::new();
    for (i, call) in calls.into_iter().enumerate() {
        let Some((method, run)) = lookup(&call.url) else {
            done[i] = Some(failed(refused(StatusCode::NOT_FOUND, "no such server function")));
            continue;
        };
        if method != call.method {
            done[i] = Some(failed(refused(StatusCode::METHOD_NOT_ALLOWED, "wrong method")));
            continue;
        }
        let Some(request) = request_of(&parts, &call) else {
            done[i] = Some(failed(refused(StatusCode::BAD_REQUEST, "bad url")));
            continue;
        };
        if !call.holes.is_empty() {
            settle(&mut done, &mut reads).await;
        }
        let args = match assemble(&call, &done) {
            Ok(args) => Bytes::from(args),
            Err(reply) => {
                done[i] = Some(failed(reply));
                continue;
            }
        };
        let paths = std::mem::take(&mut wanted[i]);
        let task = async move {
            finish(with_request(request, run(args, paths.clone())).await, paths).await
        };
        if method == Method::Get {
            reads.push((i, tokio::spawn(task)));
        } else {
            settle(&mut done, &mut reads).await;
            done[i] = Some(task.await);
        }
    }
    settle(&mut done, &mut reads).await;
    let replies: Vec<Reply> = done
        .into_iter()
        .map(|d| {
            d.map_or_else(|| refused(StatusCode::INTERNAL_SERVER_ERROR, "no answer"), |d| d.reply)
        })
        .collect();
    ([(header::CONTENT_TYPE, BATCH_CONTENT_TYPE)], batch::encode_response(&replies)).into_response()
}

fn failed(reply: Reply) -> Done {
    Done { reply, pieces: Vec::new() }
}

/// Waits for the reads started so far: a write that follows must see their effects settled, and a
/// call that takes from one of them needs its answer
async fn settle(
    done: &mut [Option<Done>],
    reads: &mut Vec<(usize, tokio::task::JoinHandle<Done>)>,
) {
    for (i, handle) in reads.drain(..) {
        done[i] = Some(match handle.await {
            Ok(d) => d,
            Err(_) => failed(refused(StatusCode::INTERNAL_SERVER_ERROR, "the function panicked")),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::codec;
    use super::*;
    use axum::body::Body;
    use axum::http::HeaderValue;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    async fn add(a: i32, b: i32) -> Result<i32, Error> {
        Ok(a + b)
    }

    async fn ping() -> Result<(), Error> {
        Ok(())
    }

    async fn fail() -> Result<u8, Error> {
        Err(Error::Status(418, "teapot".into()))
    }

    #[derive(yawn_bits::Bits, Clone, PartialEq, Debug)]
    #[bits(crate = "yawn_bits")]
    struct User {
        id: u32,
        name: String,
    }

    async fn user(id: u32) -> Result<User, Error> {
        if id == 0 {
            return Err(Error::Status(404, "no such user".into()));
        }
        Ok(User { id, name: format!("user{id}") })
    }

    async fn greet(id: u32, word: String) -> Result<String, Error> {
        Ok(format!("{word} {id}"))
    }

    async fn who() -> Result<usize, Error> {
        let headers = extract::<HeaderMap>().await?;
        Ok(headers.get("x-user").map_or(0, |v| v.len()))
    }

    static LOG: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());
    static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);

    async fn note(n: u8) -> Result<u8, Error> {
        let now = IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
        PEAK.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        LOG.lock().unwrap().push(n);
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        Ok(n)
    }

    async fn read(n: u8) -> Result<u8, Error> {
        note(n).await
    }

    async fn write(n: u8) -> Result<u8, Error> {
        note(n).await
    }

    fn app() -> axum::Router {
        let r = axum::Router::new();
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/add", Method::Post, add);
        let r = route::<codec::Json, _, _>(r, "/__yawn/fn/t/jadd", Method::Put, add);
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/ping", Method::Get, ping);
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/fail", Method::Delete, fail);
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/who", Method::Get, who);
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/user", Method::Get, user);
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/greet", Method::Post, greet);
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/read", Method::Get, read);
        let r = route::<codec::Bits, _, _>(r, "/__yawn/fn/t/write", Method::Post, write);
        batch_route(r)
    }

    fn run<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_multi_thread().enable_time().build().unwrap().block_on(f)
    }

    async fn send(
        app: &axum::Router,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> (u16, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(url);
        for (k, v) in headers {
            req = req.header(*k, HeaderValue::from_str(v).unwrap());
        }
        let res = app.clone().oneshot(req.body(Body::from(body)).unwrap()).await.unwrap();
        let status = res.status().as_u16();
        (status, axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec())
    }

    const BITS: &str = <codec::Bits as Codec<()>>::CONTENT_TYPE;

    fn bits<T: yawn_bits::Bits>(v: &T) -> Vec<u8> {
        yawn_bits::to_vec(v)
    }

    #[test]
    fn a_call_runs_in_each_codec() {
        run(async {
            let app = app();
            let (status, body) = send(
                &app,
                "POST",
                "/__yawn/fn/t/add",
                &[("content-type", BITS)],
                bits(&(2i32, 3i32)),
            )
            .await;
            assert_eq!((status, yawn_bits::from_slice::<i32>(&body)), (200, Ok(5)));
            let (status, body) = send(
                &app,
                "PUT",
                "/__yawn/fn/t/jadd",
                &[("content-type", "application/json; charset=utf-8")],
                b"[2,3]".to_vec(),
            )
            .await;
            assert_eq!((status, body.as_slice()), (200, &b"5"[..]));
        });
    }

    #[test]
    fn a_call_is_refused_without_its_content_type_or_header() {
        run(async {
            let app = app();
            let args = bits(&(2i32, 3i32));
            let wrong = [("content-type", "application/json")];
            assert_eq!(send(&app, "POST", "/__yawn/fn/t/add", &wrong, args.clone()).await.0, 415);
            assert_eq!(send(&app, "POST", "/__yawn/fn/t/add", &[], args.clone()).await.0, 415);
            assert_eq!(send(&app, "GET", "/__yawn/fn/t/ping?args=", &[], vec![]).await.0, 403);
            let marked = [("x-yawn-fn", "1")];
            let query =
                "/__yawn/fn/t/ping?args=".to_string() + &super::super::encode_args(&bits(&()));
            assert_eq!(send(&app, "GET", &query, &marked, vec![]).await.0, 200);
            assert_eq!(send(&app, "GET", "/__yawn/fn/t/ping", &marked, vec![]).await.0, 400);
        });
    }

    #[test]
    fn a_wrong_shape_is_outdated_and_garbage_is_a_bad_request() {
        run(async {
            let app = app();
            let h = [("content-type", BITS)];
            let (status, _) = send(&app, "POST", "/__yawn/fn/t/add", &h, bits(&(2u8, 3u8))).await;
            assert_eq!(status, 409);
            let mut bad = bits(&(2i32, 3i32));
            bad.truncate(bad.len() - 1);
            assert_eq!(send(&app, "POST", "/__yawn/fn/t/add", &h, bad).await.0, 400);
            assert_eq!(send(&app, "POST", "/__yawn/fn/t/add", &h, vec![]).await.0, 409);
            let (status, body) = send(&app, "DELETE", "/__yawn/fn/t/fail", &h, bits(&())).await;
            assert_eq!((status, body.as_slice()), (418, &b"teapot"[..]));
        });
    }

    fn call(method: Method, url: &str, args: Vec<u8>) -> batch::Call {
        batch::Call { method, url: url.into(), args, holes: vec![], content_type: "" }
    }

    async fn batch_of(
        app: &axum::Router,
        calls: &[batch::Call],
        headers: &[(&str, &str)],
    ) -> Vec<batch::Reply> {
        let mut h = vec![("content-type", BATCH_CONTENT_TYPE)];
        h.extend_from_slice(headers);
        let (status, body) = send(app, "POST", BATCH_URL, &h, batch::encode_request(calls)).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
        batch::decode_response(&body, calls.len()).expect("a readable answer")
    }

    #[test]
    fn a_batch_answers_every_call_by_itself() {
        run(async {
            let app = app();
            let replies = batch_of(
                &app,
                &[
                    call(Method::Post, "/__yawn/fn/t/add", bits(&(1i32, 2i32))),
                    call(Method::Get, "/__yawn/fn/t/nope", vec![]),
                    call(Method::Get, "/__yawn/fn/t/add", bits(&(1i32, 2i32))),
                    call(Method::Delete, "/__yawn/fn/t/fail", bits(&())),
                    call(Method::Post, "/__yawn/fn/t/add", bits(&(2u8, 3u8))),
                    call(Method::Get, "/__yawn/fn/t/who", bits(&())),
                ],
                &[("x-user", "bob")],
            )
            .await;
            let statuses: Vec<u16> = replies.iter().map(|r| r.status).collect();
            assert_eq!(statuses, vec![200, 404, 405, 418, 409, 200]);
            assert_eq!(yawn_bits::from_slice::<i32>(&replies[0].body), Ok(3));
            assert_eq!(replies[3].body, b"teapot");
            assert_eq!(
                yawn_bits::from_slice::<usize>(&replies[5].body),
                Ok(3),
                "extractors see the batch's headers"
            );
        });
    }

    #[test]
    fn a_batch_runs_reads_together_and_writes_in_order() {
        run(async {
            let app = app();
            LOG.lock().unwrap().clear();
            PEAK.store(0, Ordering::SeqCst);
            let calls: Vec<_> = [
                (Method::Get, "read", 1u8),
                (Method::Get, "read", 2),
                (Method::Get, "read", 3),
                (Method::Post, "write", 4),
                (Method::Post, "write", 5),
                (Method::Get, "read", 6),
            ]
            .into_iter()
            .map(|(m, f, n)| call(m, &format!("/__yawn/fn/t/{f}"), bits(&(n,))))
            .collect();
            let replies = batch_of(&app, &calls, &[]).await;
            let got: Vec<u8> =
                replies.iter().map(|r| yawn_bits::from_slice::<u8>(&r.body).unwrap()).collect();
            assert_eq!(got, vec![1, 2, 3, 4, 5, 6]);
            let log = LOG.lock().unwrap().clone();
            assert_eq!(
                &log[3..],
                &[4, 5, 6],
                "a write waits for the reads before it, and the next call for the write"
            );
            assert!(PEAK.load(Ordering::SeqCst) >= 3, "the three reads overlapped");
        });
    }

    #[test]
    fn a_batch_is_checked() {
        run(async {
            let app = app();
            let good = batch::encode_request(&[call(
                Method::Post,
                "/__yawn/fn/t/add",
                bits(&(1i32, 2i32)),
            )]);
            assert_eq!(send(&app, "POST", BATCH_URL, &[], good.clone()).await.0, 415);
            let h = [("content-type", BATCH_CONTENT_TYPE)];
            assert_eq!(send(&app, "POST", BATCH_URL, &h, vec![9, 9, 9]).await.0, 400);
            let many: Vec<_> = (0..MAX_CALLS_SERVED + 1)
                .map(|_| call(Method::Post, "/__yawn/fn/t/add", bits(&(1i32, 2i32))))
                .collect();
            assert_eq!(
                send(&app, "POST", BATCH_URL, &h, batch::encode_request(&many)).await.0,
                400
            );
            assert_eq!(send(&app, "GET", BATCH_URL, &h, vec![]).await.0, 405);
            assert_eq!(send(&app, "POST", BATCH_URL, &h, good).await.0, 200);
        });
    }

    fn greet_from(from: u32, path: &[u32], schema: u64, method: Method) -> batch::Call {
        let mut args = yawn_bits::header::<(u32, String)>().to_vec();
        let at = args.len();
        yawn_bits::put_value(&"hello".to_string(), &mut args);
        batch::Call {
            method,
            url: "/__yawn/fn/t/greet".into(),
            args,
            holes: vec![batch::Hole {
                at,
                from,
                path: path.to_vec(),
                schema: yawn_bits::folded_schema(schema),
            }],
            content_type: "",
        }
    }

    #[test]
    fn a_call_can_take_its_argument_from_an_earlier_answer() {
        use yawn_bits::{Bits, Pipe};
        run(async {
            let app = app();
            let id = User::fields().id;
            let first = call(Method::Get, "/__yawn/fn/t/user", {
                let mut a = yawn_bits::header::<(u32,)>().to_vec();
                yawn_bits::put_value(&7u32, &mut a);
                a
            });
            let piped = greet_from(0, id.path(), u32::SCHEMA, Method::Post);
            let replies = batch_of(&app, &[first.clone(), piped], &[]).await;
            assert_eq!(replies[0].status, 200);
            assert_eq!(replies[1].status, 200, "{}", String::from_utf8_lossy(&replies[1].body));
            assert_eq!(
                yawn_bits::from_slice::<String>(&replies[1].body),
                Ok("hello 7".to_string())
            );

            let wrong_type = greet_from(0, id.path(), String::SCHEMA, Method::Post);
            assert_eq!(batch_of(&app, &[first.clone(), wrong_type], &[]).await[1].status, 409);
            let no_field = greet_from(0, &[5], u32::SCHEMA, Method::Post);
            assert_eq!(batch_of(&app, &[first.clone(), no_field], &[]).await[1].status, 424);

            let missing = call(Method::Get, "/__yawn/fn/t/user", {
                let mut a = yawn_bits::header::<(u32,)>().to_vec();
                yawn_bits::put_value(&0u32, &mut a);
                a
            });
            let replies = batch_of(
                &app,
                &[missing, greet_from(0, id.path(), u32::SCHEMA, Method::Post)],
                &[],
            )
            .await;
            assert_eq!(replies.iter().map(|r| r.status).collect::<Vec<_>>(), vec![404, 424]);
        });
    }
}
