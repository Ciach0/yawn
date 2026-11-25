use std::collections::BTreeMap;

/// The manifest's file name, in the `public` directory of a build
pub const MANIFEST_FILE: &str = "manifest.json";

/// Length of the hash in a file name
pub const HASH_LEN: usize = 12;

/// Written name -> served name, e.g. `/assets/logo.webp` -> `/assets/logo.3fa9c1d2e4b5.webp`
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    map: BTreeMap<String, String>,
}

impl Manifest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: impl Into<String>, served: impl Into<String>) {
        self.map.insert(path.into(), served.into());
    }

    pub fn get(&self, path: &str) -> Option<&str> {
        self.map.get(path).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// `{"/assets/a.png":"/assets/a.<hash>.png",...}`; safe inside an inline `<script>` too
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        for (i, (k, v)) in self.map.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&crate::protocol::json_string(k));
            out.push(':');
            out.push_str(&crate::protocol::json_string(v));
        }
        out.push('}');
        out
    }

    /// Reads what [`Manifest::to_json`] wrote: one flat object of strings
    pub fn parse(json: &str) -> Option<Manifest> {
        let mut p = Parser { s: json.as_bytes(), i: 0 };
        let mut m = Manifest::new();
        p.ws();
        p.eat(b'{')?;
        p.ws();
        if p.peek() == Some(b'}') {
            return Some(m);
        }
        loop {
            p.ws();
            let k = p.string()?;
            p.ws();
            p.eat(b':')?;
            p.ws();
            let v = p.string()?;
            m.insert(k, v);
            p.ws();
            match p.next()? {
                b',' => {}
                b'}' => return Some(m),
                _ => return None,
            }
        }
    }

    /// `value` with its path replaced by the served one; a query or fragment is kept
    pub fn resolve(&self, value: &str) -> String {
        if !value.starts_with('/') || value.starts_with("//") {
            return value.to_string();
        }
        let end = value.find(['?', '#']).unwrap_or(value.len());
        match self.get(&value[..end]) {
            Some(served) => format!("{served}{}", &value[end..]),
            None => value.to_string(),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.i += 1;
        Some(b)
    }
    fn ws(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    fn eat(&mut self, b: u8) -> Option<()> {
        (self.next()? == b).then_some(())
    }
    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut bytes = Vec::new();
        loop {
            match self.next()? {
                b'"' => return String::from_utf8(bytes).ok(),
                b'\\' => match self.next()? {
                    b'"' => bytes.push(b'"'),
                    b'\\' => bytes.push(b'\\'),
                    b'/' => bytes.push(b'/'),
                    b'n' => bytes.push(b'\n'),
                    b'r' => bytes.push(b'\r'),
                    b't' => bytes.push(b'\t'),
                    b'u' => {
                        let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                        self.i += 4;
                        let c = char::from_u32(u32::from_str_radix(hex, 16).ok()?)
                            .unwrap_or('\u{fffd}');
                        let mut buf = [0u8; 4];
                        bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                    _ => return None,
                },
                b => bytes.push(b),
            }
        }
    }
}

/// Whether `path` names a file whose name carries a content hash (`app.3fa9c1d2e4b5.js`)
pub fn is_hashed(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut parts = name.rsplitn(3, '.');
    let (_ext, hash, stem) = (parts.next(), parts.next(), parts.next());
    stem.is_some()
        && hash.is_some_and(|h| h.len() == HASH_LEN && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `name.ext` -> `name.<hash>.ext` (`name` -> `name.<hash>` when there is no extension)
pub fn hashed_name(path: &str, hash: &str) -> String {
    let (dir, name) = path.rsplit_once('/').map_or(("", path), |(d, n)| (d, n));
    let name = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}.{hash}.{ext}"),
        _ => format!("{name}.{hash}"),
    };
    if dir.is_empty() && !path.contains('/') { name } else { format!("{dir}/{name}") }
}

/// 12 hex characters from the content (FNV-1a over 64 bits, folded): stable across builds and
/// machines, which is what a cache key needs
pub fn content_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    // a second pass with the length mixed in keeps short files with equal prefixes apart
    let mut g: u64 = h ^ (bytes.len() as u64).wrapping_mul(0x9e3779b97f4a7c15);
    g ^= g >> 33;
    g = g.wrapping_mul(0xff51afd7ed558ccd);
    g ^= g >> 33;
    format!("{g:016x}")[..HASH_LEN].to_string()
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use super::Manifest;
    use std::sync::OnceLock;

    static INSTALLED: OnceLock<Manifest> = OnceLock::new();

    pub fn install(m: Manifest) {
        let _ = INSTALLED.set(m);
    }

    pub fn resolve(value: &str) -> String {
        match INSTALLED.get() {
            Some(m) => m.resolve(value),
            None => value.to_string(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod store {
    use crate::dom;

    /// The manifest the server put in the page (`window.__yawnAssets`)
    pub fn resolve(value: &str) -> String {
        if !value.starts_with('/') || value.starts_with("//") {
            return value.to_string();
        }
        let end = value.find(['?', '#']).unwrap_or(value.len());
        let served = dom::window()
            .and_then(|w| dom::get_object(&w, "__yawnAssets"))
            .and_then(|all| dom::get_string(&all, &value[..end]));
        match served {
            Some(s) => format!("{s}{}", &value[end..]),
            None => value.to_string(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use store::install;

/// The URL to use for `path` (`/assets/logo.webp`): the hashed one in production, `path` itself in
/// development and for anything that isn't an asset
pub fn asset(path: &str) -> String {
    store::resolve(path)
}

pub(crate) fn for_attr(value: &str) -> std::borrow::Cow<'_, str> {
    if value.starts_with("/assets/") {
        std::borrow::Cow::Owned(asset(value))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_and_resolves() {
        let mut m = Manifest::new();
        m.insert("/assets/a b.png", "/assets/a b.0123456789ab.png");
        m.insert("/pkg/app.js", "/pkg/app.aaaaaaaaaaaa.js");
        let back = Manifest::parse(&m.to_json()).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.resolve("/pkg/app.js"), "/pkg/app.aaaaaaaaaaaa.js");
        assert_eq!(back.resolve("/pkg/app.js?x=1#h"), "/pkg/app.aaaaaaaaaaaa.js?x=1#h");
        assert_eq!(back.resolve("/other.js"), "/other.js");
        assert_eq!(back.resolve("https://x/pkg/app.js"), "https://x/pkg/app.js");
        assert_eq!(back.resolve("//cdn/pkg/app.js"), "//cdn/pkg/app.js");
        assert_eq!(Manifest::parse("{}").unwrap(), Manifest::new());
        assert_eq!(
            Manifest::parse(" { \"/a\" : \"\\u003cb\\u003e\" } ").unwrap().get("/a"),
            Some("<b>")
        );
        assert!(Manifest::parse("[1]").is_none() && Manifest::parse("{\"a\":1}").is_none());
    }

    #[test]
    fn names_carry_the_hash_and_are_recognised() {
        assert_eq!(
            hashed_name("/assets/logo.webp", "0123456789ab"),
            "/assets/logo.0123456789ab.webp"
        );
        assert_eq!(hashed_name("/assets/a.b.css", "0123456789ab"), "/assets/a.b.0123456789ab.css");
        assert_eq!(hashed_name("/assets/LICENSE", "0123456789ab"), "/assets/LICENSE.0123456789ab");
        assert!(is_hashed("/assets/logo.0123456789ab.webp"));
        assert!(is_hashed("/pkg/app_bg.0123456789ab.wasm"));
        assert!(!is_hashed("/assets/logo.webp"));
        assert!(!is_hashed("/assets/jquery.min.js"));
        assert!(!is_hashed("/assets/v1.2.3.js"));
    }

    #[test]
    fn hash_follows_the_content() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
        assert_ne!(content_hash(b""), content_hash(b"\0"));
        assert_eq!(content_hash(b"x").len(), HASH_LEN);
    }

    #[test]
    fn without_a_manifest_paths_stay() {
        assert_eq!(asset("/assets/x.png"), "/assets/x.png");
    }
}
