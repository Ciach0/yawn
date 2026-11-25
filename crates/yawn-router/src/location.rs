//! Where the application is: the URL split into path, query and hash.

/// The part of the URL the router looks at
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Location {
    /// Always starts with `/`; no query or hash
    pub path: String,
    /// Without the leading `?`
    pub query: String,
    /// Without the leading `#`
    pub hash: String,
}

impl Location {
    /// Splits `url` (`/users/3?tab=posts#top`)
    pub fn parse(url: &str) -> Location {
        let (rest, hash) = url.split_once('#').unwrap_or((url, ""));
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
        Location { path, query: query.to_string(), hash: hash.to_string() }
    }

    /// Value of the first `name=` in the query, percent-decoded
    pub fn query_param(&self, name: &str) -> Option<String> {
        self.query.split('&').filter(|p| !p.is_empty()).find_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(k, true) == name).then(|| decode(v, true))
        })
    }

    /// The path as decoded segments: `/a/b%20c/` is `["a", "b c"]`
    pub fn segments(&self) -> Vec<String> {
        self.path.split('/').filter(|s| !s.is_empty()).map(|s| decode(s, false)).collect()
    }

    /// `path?query#hash`
    pub fn url(&self) -> String {
        let mut s = self.path.clone();
        if !self.query.is_empty() {
            s.push('?');
            s.push_str(&self.query);
        }
        if !self.hash.is_empty() {
            s.push('#');
            s.push_str(&self.hash);
        }
        s
    }
}

pub(crate) fn decode(s: &str, plus: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len()
                && (b[i + 1] as char).is_ascii_hexdigit()
                && (b[i + 2] as char).is_ascii_hexdigit() =>
            {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            }
            b'+' if plus => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_parts() {
        let l = Location::parse("/users/3?tab=posts&q=a%20b+c#top");
        assert_eq!(
            (l.path.as_str(), l.query.as_str(), l.hash.as_str()),
            ("/users/3", "tab=posts&q=a%20b+c", "top")
        );
        assert_eq!(l.query_param("tab").as_deref(), Some("posts"));
        assert_eq!(l.query_param("q").as_deref(), Some("a b c"));
        assert_eq!(l.query_param("nope"), None);
        assert_eq!(l.url(), "/users/3?tab=posts&q=a%20b+c#top");
        assert_eq!(Location::parse("about").path, "/about");
    }

    #[test]
    fn segments_are_decoded_and_empty_ones_dropped() {
        assert_eq!(Location::parse("/a//b%20c/").segments(), ["a", "b c"]);
        assert!(Location::parse("/").segments().is_empty());
        assert_eq!(decode("100%", false), "100%");
        assert_eq!(decode("%zz", false), "%zz");
    }
}
