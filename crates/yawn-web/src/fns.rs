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

/// The endpoint of the function `name` in the module `module` (a `module_path!()`). The crate name
/// is dropped, so the URL is the same whichever crate the dev build moved the module to.
#[doc(hidden)]
pub fn url(module: &str, name: &str) -> String {
    let path = module.split_once("::").map_or("", |(_, rest)| rest).replace("::", "/");
    if path.is_empty() { format!("/__yawn/fn/{name}") } else { format!("/__yawn/fn/{path}/{name}") }
}

/// Async functions that run on the server and that the client calls like any other: in the
/// client, each one sends its arguments to the server as JSON and returns the answer.
///
/// ```ignore
/// // src/fns/todos.rs
/// yawn::server_fn! {
///     pub async fn add(title: String) -> usize {
///         crate::server::db().insert(title)
///     }
/// }
/// ```
///
/// `todos::add(title).await` is a `Result<usize, yawn::fns::Error>` on both sides, so a component
/// that calls it compiles for SSR too. The blocks only work in files of `src/fns/`, which the
/// server routes. Arguments and the answer are `serde` types; the arguments come from the browser,
/// so check them like any request. The body is compiled for the server only, so it can use
/// server-only code; the rest of the file is compiled for both.
#[macro_export]
macro_rules! server_fn {
    ($(
        $(#[$attr:meta])*
        $vis:vis async fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)? $body:block
    )*) => {$(
        #[cfg(not(target_arch = "wasm32"))]
        $(#[$attr])*
        $vis async fn $name($($arg: $ty),*)
            -> ::core::result::Result<$crate::__server_fn_ret!($($ret)?), $crate::fns::Error>
        {
            ::core::result::Result::Ok(async move $body.await)
        }

        #[cfg(target_arch = "wasm32")]
        $(#[$attr])*
        $vis async fn $name($($arg: $ty),*)
            -> ::core::result::Result<$crate::__server_fn_ret!($($ret)?), $crate::fns::Error>
        {
            $crate::fns::call(::core::module_path!(), ::core::stringify!($name), &($($arg,)*)).await
        }
    )*};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __server_fn_ret {
    () => {
        ()
    };
    ($t:ty) => {
        $t
    };
}

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub async fn call<A: serde::Serialize, T: serde::de::DeserializeOwned>(
    module: &str,
    name: &str,
    args: &A,
) -> Result<T, Error> {
    use crate::dom::{FetchError, FetchRequest, fetch_bytes};
    let body = serde_json::to_vec(args).map_err(|e| Error::Body(e.to_string()))?;
    let headers = [("content-type".to_string(), "application/json".to_string())];
    let r = fetch_bytes(FetchRequest {
        url: &url(module, name),
        method: "POST",
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
    use super::Error;
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode, header};
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

    /// Answers a call: the JSON content type is required because a cross-site form can't send it,
    /// so a page elsewhere can't call a function with the user's cookies
    pub(super) async fn serve<A, F: Handler<A>>(
        f: F,
        headers: &HeaderMap,
        body: Bytes,
    ) -> Response {
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
    pub fn handler<A: 'static, F: Handler<A>>(f: F) -> axum::routing::MethodRouter {
        axum::routing::post(move |headers: HeaderMap, body: Bytes| {
            let f = f.clone();
            async move { serve(f, &headers, body).await }
        })
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

    mod declared {
        crate::server_fn! {
            /// Adds
            pub async fn add(a: i32, b: i32) -> i32 { a + b }
            pub(crate) async fn ping() {}
            pub async fn early(n: u8,) -> Option<u8> {
                if n == 0 {
                    return None;
                }
                let less = n.checked_sub(1)?;
                Some(less * 2)
            }
        }
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
            let call = |f, headers: &HeaderMap, body: &'static str| {
                let headers = headers.clone();
                async move {
                    let r = serve::serve(f, &headers, body.into()).await;
                    let status = r.status().as_u16();
                    let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
                    (status, String::from_utf8(body.to_vec()).unwrap())
                }
            };
            assert_eq!(call(declared::add, &json, "[2,3]").await, (200, "5".into()));
            assert_eq!(call(declared::add, &json, "[2]").await.0, 400);
            assert_eq!(call(declared::add, &HeaderMap::new(), "[2,3]").await.0, 415);
            let r = serve::serve(declared::ping, &json, "null".into()).await;
            assert_eq!(r.status().as_u16(), 200);
            let r = serve::serve(declared::early, &json, "[4]".into()).await;
            assert_eq!(axum::body::to_bytes(r.into_body(), 64).await.unwrap(), "6");
        });
    }
}
