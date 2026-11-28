//! The runtime of server functions in `.rsx` files (`#[get]`, `#[post]`, `#[put]`, `#[patch]` and
//! `#[delete]` on a `fn`): the client's call and the server's route. The compiler writes both
//! versions of each function (see yawn-rsx's README).

use std::fmt;

/// Why calling a server function failed. On the server the call is a plain function call and
/// always succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No answer: offline, timed out, refused by the browser
    Network(String),
    /// The server answered with an error status; the body says why (404 is a function the server
    /// doesn't have, as when the client is older than the server)
    Status(u16, String),
    /// The arguments or the answer don't fit their types
    Body(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Network(m) => write!(f, "server function unreachable: {m}"),
            Error::Status(code, m) => write!(f, "server function failed ({code}): {m}"),
            Error::Body(m) => write!(f, "server function data unreadable: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// The HTTP method of a server function: `#[get]` reads, the others write. A `GET` carries its
/// arguments in the URL and the others in a JSON body.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    #[cfg(any(target_arch = "wasm32", test))]
    fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }
}

/// The header a `GET` call carries: a page elsewhere can't add it without the server's consent
/// (a cross-site request with a custom header needs a preflight), so it can't read through the
/// user's cookies. The other methods have the JSON content type for the same purpose.
#[doc(hidden)]
pub const CALL_HEADER: &str = "x-yawn-fn";

/// The arguments of a `GET` call as the value of `?args=`: every byte but the unreserved ones as
/// `%XX`
#[cfg(any(target_arch = "wasm32", test))]
fn encode_args(json: &[u8]) -> String {
    let mut out = String::with_capacity(json.len() * 2);
    for &b in json {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The value of `args` in a query string, undone from `%XX` (and `+`, which a form would write)
#[cfg(any(all(feature = "server", not(target_arch = "wasm32")), test))]
fn decode_args(query: &str) -> Option<Vec<u8>> {
    let raw = query.split('&').find_map(|pair| pair.strip_prefix("args="))?.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'%' => {
                let hex = std::str::from_utf8(raw.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

/// The endpoint of the function `name` in the module `module` (a `module_path!()`). The crate name
/// is dropped, so the URL is the same whichever crate the dev build moved the module to.
#[doc(hidden)]
pub fn url(module: &str, name: &str) -> String {
    let path = module.split_once("::").map_or("", |(_, rest)| rest).replace("::", "/");
    if path.is_empty() { format!("/__yawn/fn/{name}") } else { format!("/__yawn/fn/{path}/{name}") }
}

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub async fn call<A: serde::Serialize, T: serde::de::DeserializeOwned>(
    method: Method,
    module: &str,
    name: &str,
    args: &A,
) -> Result<T, Error> {
    use crate::dom::{FetchError, FetchRequest, fetch_bytes};
    let json = serde_json::to_vec(args).map_err(|e| Error::Body(e.to_string()))?;
    let (url, headers, body) = if method == Method::Get {
        let url = format!("{}?args={}", url(module, name), encode_args(&json));
        (url, [(CALL_HEADER.to_string(), "1".to_string())], Vec::new())
    } else {
        let headers = [("content-type".to_string(), "application/json".to_string())];
        (url(module, name), headers, json)
    };
    let r = fetch_bytes(FetchRequest {
        url: &url,
        method: method.as_str(),
        headers: &headers,
        body: &body,
        timeout_ms: 30_000,
    })
    .await
    .map_err(|e| match e {
        FetchError::Timeout => Error::Network("timed out".into()),
        FetchError::Header(m) | FetchError::Network(m) => Error::Network(m),
    })?;
    if !(200..300).contains(&r.status) {
        return Err(Error::Status(r.status, String::from_utf8_lossy(&r.body).into_owned()));
    }
    serde_json::from_slice(&r.body).map_err(|e| Error::Body(e.to_string()))
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use serve::{Handler, handler};

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod serve {
    use super::{CALL_HEADER, Error, Method, decode_args};
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode, Uri, header};
    use axum::response::{IntoResponse, Response};

    /// A server function as the router calls it, `Args` being the tuple of its argument types
    #[doc(hidden)]
    pub trait Handler<Args>: Clone + Send + Sync + 'static {
        fn run(self, body: Bytes) -> impl Future<Output = Response> + Send;
    }

    fn answer<T: serde::Serialize>(out: Result<T, Error>) -> Response {
        match out.map(|v| serde_json::to_vec(&v)) {
            Ok(Ok(json)) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
            Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }

    macro_rules! handlers {
        ($($a:ident: $t:ident),*) => {
            impl<F, Fut, T, $($t),*> Handler<($($t,)*)> for F
            where
                F: Fn($($t),*) -> Fut + Clone + Send + Sync + 'static,
                Fut: Future<Output = Result<T, Error>> + Send,
                T: serde::Serialize,
                $($t: serde::de::DeserializeOwned + Send,)*
            {
                async fn run(self, body: Bytes) -> Response {
                    match serde_json::from_slice::<($($t,)*)>(&body) {
                        Ok(($($a,)*)) => answer(self($($a),*).await),
                        Err(e) => (StatusCode::BAD_REQUEST, format!("cannot read the arguments: {e}"))
                            .into_response(),
                    }
                }
            }
        };
    }
    handlers!();
    handlers!(a: A);
    handlers!(a: A, b: B);
    handlers!(a: A, b: B, c: C);
    handlers!(a: A, b: B, c: C, d: D);
    handlers!(a: A, b: B, c: C, d: D, e: E);
    handlers!(a: A, b: B, c: C, d: D, e: E, f: G);
    handlers!(a: A, b: B, c: C, d: D, e: E, f: G, g: H);
    handlers!(a: A, b: B, c: C, d: D, e: E, f: G, g: H, h: I);

    /// Answers a call. A `GET` needs the `x-yawn-fn` header and the others the JSON content type:
    /// a cross-site page can send neither, so it can't call a function with the user's cookies
    pub(super) async fn serve<A, F: Handler<A>>(
        method: Method,
        f: F,
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
                Some(args) => f.run(Bytes::from(args)).await,
                None => (StatusCode::BAD_REQUEST, "the call has no readable args").into_response(),
            };
        }
        let json = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "a server function takes application/json",
            )
                .into_response();
        }
        f.run(body).await
    }

    /// The route of one server function
    #[doc(hidden)]
    pub fn handler<A: 'static, F: Handler<A>>(method: Method, f: F) -> axum::routing::MethodRouter {
        macro_rules! route {
            ($verb:ident) => {
                axum::routing::$verb(move |headers: HeaderMap, uri: Uri, body: Bytes| {
                    let f = f.clone();
                    async move { serve(method, f, &headers, uri.query(), body).await }
                })
            };
        }
        match method {
            Method::Get => route!(get),
            Method::Post => route!(post),
            Method::Put => route!(put),
            Method::Patch => route!(patch),
            Method::Delete => route!(delete),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_leave_the_crate_out() {
        assert_eq!(url("yawn_app::fns::todos", "add"), "/__yawn/fn/fns/todos/add");
        assert_eq!(
            url("yawn_split_app_fns::fns::admin::users", "ban"),
            "/__yawn/fn/fns/admin/users/ban"
        );
        assert_eq!(url("app", "ping"), "/__yawn/fn/ping");
    }

    // what the compiler writes on the server for server functions
    mod declared {
        use super::Error;

        pub async fn add(a: i32, b: i32) -> Result<i32, Error> {
            Ok(async move { a + b }.await)
        }

        pub async fn ping() -> Result<(), Error> {
            Ok(())
        }

        pub async fn early(n: u8) -> Result<Option<u8>, Error> {
            Ok(async move {
                if n == 0 {
                    return None;
                }
                let less = n.checked_sub(1)?;
                Some(less * 2)
            }
            .await)
        }
    }

    #[test]
    fn get_arguments_survive_the_query_string() {
        for json in [&b"[]"[..], br#"[1,"a b&c=d%",{"k":["\u00e9"]}]"#, "[\"zażółć\"]".as_bytes()]
        {
            let query = format!("x=1&args={}&y=2", encode_args(json));
            assert!(query.is_ascii() && !query["x=1&args=".len()..].contains(['"', ' ', '{']));
            assert_eq!(decode_args(&query).as_deref(), Some(json));
        }
        assert_eq!(decode_args("args=%5B1%2C2%5D+"), Some(b"[1,2] ".to_vec()));
        assert_eq!(decode_args("other=1"), None);
        assert_eq!(decode_args("args=%5"), None);
        assert_eq!(decode_args("args=%zz"), None);
        assert_eq!(Method::Patch.as_str(), "PATCH");
    }

    #[cfg(all(feature = "server", not(target_arch = "wasm32")))]
    #[test]
    fn the_server_runs_calls_from_json() {
        use axum::http::{HeaderMap, HeaderValue, header};
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            assert_eq!(declared::add(2, 3).await, Ok(5));
            assert_eq!(declared::early(0).await, Ok(None));
            assert_eq!(declared::ping().await, Ok(()));
            let mut json = HeaderMap::new();
            json.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            );
            let mut marked = HeaderMap::new();
            marked.insert(CALL_HEADER, HeaderValue::from_static("1"));
            let call = |method,
                        f,
                        headers: &HeaderMap,
                        query: Option<&'static str>,
                        body: &'static str| {
                let headers = headers.clone();
                async move {
                    let r = serve::serve(method, f, &headers, query, body.into()).await;
                    let status = r.status().as_u16();
                    let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
                    (status, String::from_utf8(body.to_vec()).unwrap())
                }
            };
            for method in [Method::Post, Method::Put, Method::Patch, Method::Delete] {
                let sum = call(method, declared::add, &json, None, "[2,3]").await;
                assert_eq!(sum, (200, "5".into()), "{method:?}");
                assert_eq!(call(method, declared::add, &json, None, "[2]").await.0, 400);
                assert_eq!(
                    call(method, declared::add, &HeaderMap::new(), None, "[2,3]").await.0,
                    415
                );
                assert_eq!(call(method, declared::add, &marked, None, "[2,3]").await.0, 415);
            }
            let query = Some("args=%5B2%2C3%5D");
            assert_eq!(
                call(Method::Get, declared::add, &marked, query, "").await,
                (200, "5".into())
            );
            assert_eq!(call(Method::Get, declared::add, &json, query, "").await.0, 403);
            assert_eq!(call(Method::Get, declared::add, &marked, None, "").await.0, 400);
            assert_eq!(
                call(Method::Get, declared::add, &marked, Some("args=%5B2%5D"), "").await.0,
                400
            );
            let r = serve::serve(Method::Post, declared::ping, &json, None, "null".into()).await;
            assert_eq!(r.status().as_u16(), 200);
            let r = serve::serve(Method::Post, declared::early, &json, None, "[4]".into()).await;
            assert_eq!(axum::body::to_bytes(r.into_body(), 64).await.unwrap(), "6");
        });
    }

    #[cfg(all(feature = "server", not(target_arch = "wasm32")))]
    #[test]
    fn every_method_makes_a_route() {
        // axum checks the routes as they are added
        let _ = axum::Router::<()>::new()
            .route("/get", handler(Method::Get, declared::ping))
            .route("/post", handler(Method::Post, declared::ping))
            .route("/put", handler(Method::Put, declared::ping))
            .route("/patch", handler(Method::Patch, declared::ping))
            .route("/delete", handler(Method::Delete, declared::ping));
    }
}
