use std::fmt;
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::install_crypto_provider;
#[cfg(not(target_arch = "wasm32"))]
use native::send;
#[cfg(target_arch = "wasm32")]
use web::send;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
        }
    }
}

/// Why a request failed
#[derive(Debug)]
pub enum Error {
    /// The URL isn't usable (`http://` or `https://` only)
    Url(String),
    /// Connecting, TLS or reading the response failed
    Network(String),
    /// No answer in time
    Timeout,
    /// The body isn't what [`Response::json`] or [`Response::text`] expected
    Body(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Url(m) => write!(f, "bad url: {m}"),
            Error::Network(m) => write!(f, "request failed: {m}"),
            Error::Timeout => write!(f, "request timed out"),
            Error::Body(m) => write!(f, "unreadable response: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// A request being built; nothing is sent until [`send`](RequestBuilder::send)
#[derive(Clone, Debug)]
pub struct RequestBuilder {
    pub(crate) method: Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) timeout: Duration,
}

pub fn get(url: impl Into<String>) -> RequestBuilder {
    RequestBuilder::new(Method::Get, url)
}
pub fn post(url: impl Into<String>) -> RequestBuilder {
    RequestBuilder::new(Method::Post, url)
}
pub fn put(url: impl Into<String>) -> RequestBuilder {
    RequestBuilder::new(Method::Put, url)
}
pub fn patch(url: impl Into<String>) -> RequestBuilder {
    RequestBuilder::new(Method::Patch, url)
}
pub fn delete(url: impl Into<String>) -> RequestBuilder {
    RequestBuilder::new(Method::Delete, url)
}

impl RequestBuilder {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        RequestBuilder {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: Duration::from_secs(30),
        }
    }

    /// Adds a header (a second one with the same name replaces the first)
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// `Authorization: Bearer <token>`
    pub fn bearer(self, token: &str) -> Self {
        self.header("authorization", &format!("Bearer {token}"))
    }

    /// Query pairs, percent-encoded and added to the URL
    pub fn query(mut self, pairs: &[(&str, &str)]) -> Self {
        if !pairs.is_empty() {
            self.url.push(if self.url.contains('?') { '&' } else { '?' });
            self.url.push_str(&encode_pairs(pairs));
        }
        self
    }

    /// A `application/x-www-form-urlencoded` body
    pub fn form(mut self, pairs: &[(&str, &str)]) -> Self {
        self.body = encode_pairs(pairs).into_bytes();
        self.header("content-type", "application/x-www-form-urlencoded")
    }

    /// A JSON body
    pub fn json<T: serde::Serialize>(mut self, value: &T) -> Self {
        // a type that can't be serialized is a bug in the caller, surfaced on send as a bad body
        self.body = serde_json::to_vec(value).unwrap_or_default();
        self.header("content-type", "application/json")
    }

    /// A raw body; set `Content-Type` yourself
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn send(self) -> Result<Response, Error> {
        if !(self.url.starts_with("https://")
            || self.url.starts_with("http://")
            || cfg!(target_arch = "wasm32"))
        {
            return Err(Error::Url(format!("{} (http:// or https:// only)", self.url)));
        }
        send(self).await
    }
}

/// An answer, body included
#[derive(Clone, Debug)]
pub struct Response {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Status 200-299
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// First header with this name (case-insensitive)
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    pub fn text(&self) -> Result<String, Error> {
        String::from_utf8(self.body.clone()).map_err(|e| Error::Body(e.to_string()))
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        serde_json::from_slice(&self.body).map_err(|e| Error::Body(e.to_string()))
    }

    /// The response itself when the status is 2xx, otherwise an error naming the status
    pub fn error_for_status(self) -> Result<Response, Error> {
        if self.ok() { Ok(self) } else { Err(Error::Network(format!("status {}", self.status))) }
    }
}

fn encode_pairs(pairs: &[(&str, &str)]) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_query_form_and_headers() {
        let r = get("https://x.test/a")
            .query(&[("q", "a b&c")])
            .query(&[("n", "1")])
            .header("Accept", "x")
            .header("accept", "y");
        assert_eq!(r.url, "https://x.test/a?q=a+b%26c&n=1");
        assert_eq!(r.headers, vec![("accept".to_string(), "y".to_string())]);
        let r = post("https://x.test").form(&[("code", "é=")]).bearer("t");
        assert_eq!(r.body, b"code=%C3%A9%3D");
        assert!(
            r.headers
                .iter()
                .any(|(n, v)| n == "content-type" && v == "application/x-www-form-urlencoded")
        );
        assert!(r.headers.iter().any(|(n, v)| n == "authorization" && v == "Bearer t"));
    }

    #[test]
    fn response_accessors() {
        let r = Response {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: br#"{"a":1}"#.to_vec(),
        };
        assert!(r.ok());
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.json::<serde_json::Value>().unwrap()["a"], 1);
        assert!(Response { status: 404, ..r }.error_for_status().is_err());
    }
}
