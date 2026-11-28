//! The runtime of server functions in `.rsx` files (`#[get]`, `#[post]`, `#[put]`, `#[patch]` and
//! `#[delete]` on a `fn`): the client's call and the server's route. The compiler writes both
//! versions of each function (see yawn-rsx's README).

use std::fmt;

pub mod batch;
pub mod codec;

#[cfg(target_arch = "wasm32")]
mod client;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod serve;

#[cfg(target_arch = "wasm32")]
pub use client::call;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use serve::{Handler, batch_route, extract, route, with_request};

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
    /// The server has a newer version of a type than this page: reload the page
    Outdated,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Network(m) => write!(f, "server function unreachable: {m}"),
            Error::Status(code, m) => write!(f, "server function failed ({code}): {m}"),
            Error::Body(m) => write!(f, "server function data unreadable: {m}"),
            Error::Outdated => f.write_str("this page is older than the server: reload it"),
        }
    }
}

impl std::error::Error for Error {}

/// The HTTP method of a server function: `#[get]` reads, the others write. A `GET` carries its
/// arguments in the URL and the others in the body.
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

/// The status of a call whose arguments were written for another version of a type
pub(crate) const OUTDATED: u16 = 409;

/// The header a `GET` call carries: a page elsewhere can't add it without the server's consent
/// (a cross-site request with a custom header needs a preflight), so it can't read through the
/// user's cookies. The other methods have the content type of their codec for the same purpose.
#[doc(hidden)]
pub const CALL_HEADER: &str = "x-yawn-fn";

/// The arguments of a `GET` call as the value of `?args=`: every byte but the unreserved ones as
/// `%XX`
#[cfg(any(target_arch = "wasm32", test))]
fn encode_args(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
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

    #[test]
    fn get_arguments_survive_the_query_string() {
        for bytes in [
            &b"[]"[..],
            br#"[1,"a b&c=d%",{"k":["\u00e9"]}]"#,
            "[\"zażółć\"]".as_bytes(),
            &[0, 255, 128, 37, 38],
        ] {
            let query = format!("x=1&args={}&y=2", encode_args(bytes));
            assert!(query.is_ascii() && !query["x=1&args=".len()..].contains(['"', ' ', '{']));
            assert_eq!(decode_args(&query).as_deref(), Some(bytes));
        }
        assert_eq!(decode_args("args=%5B1%2C2%5D+"), Some(b"[1,2] ".to_vec()));
        assert_eq!(decode_args("other=1"), None);
        assert_eq!(decode_args("args=%5"), None);
        assert_eq!(decode_args("args=%zz"), None);
        assert_eq!(Method::Patch.as_str(), "PATCH");
    }
}
