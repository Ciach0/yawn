/// JSON string literal that's also safe inside an inline `<script>`
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A command from the CLI on stdin, delivered to every connected browser
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Reload {
        rev: u64,
    },
    Patch {
        rev: u64,
    },
    /// Only modules were built: the page loads them and builds its components again, the application
    /// itself stays
    Module {
        rev: u64,
    },
    Css {
        rev: u64,
        href: String,
    },
    Error {
        rev: u64,
        text: String,
    },
    /// How far the build is: `done` of `total` units (crates) and what's compiling now
    Progress {
        rev: u64,
        done: u32,
        total: u32,
        label: String,
    },
    /// The server binary is older than the components: stop rendering pages on the server, the
    /// browser renders them
    NoSsr {
        rev: u64,
    },
    /// A watched data file changed (`[dev] watch`): the server runs its change hooks
    Data {
        rev: u64,
        path: String,
    },
}

/// Wire format, one line each: `reload <rev>`, `css <rev> <href>`, `error <rev> <text>`, `progress
/// <rev> <done> <total> <label>` where the text escapes `\` as `\\` and line breaks as `\n`
pub fn parse_control(line: &str) -> Option<Control> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (cmd, rest) = line.split_once(' ')?;
    let (rev, arg) = rest.split_once(' ').map_or((rest, ""), |(a, b)| (a, b));
    let rev: u64 = rev.parse().ok()?;
    match cmd {
        "reload" => Some(Control::Reload { rev }),
        "patch" => Some(Control::Patch { rev }),
        "module" => Some(Control::Module { rev }),
        "nossr" => Some(Control::NoSsr { rev }),
        "data" if !arg.is_empty() => Some(Control::Data { rev, path: unescape(arg) }),
        "css" if !arg.is_empty() => Some(Control::Css { rev, href: arg.to_string() }),
        "error" => Some(Control::Error { rev, text: unescape(arg) }),
        "progress" => {
            let mut it = arg.splitn(3, ' ');
            let done = it.next()?.parse().ok()?;
            let total = it.next()?.parse().ok()?;
            Some(Control::Progress { rev, done, total, label: unescape(it.next().unwrap_or("")) })
        }
        _ => None,
    }
}

pub fn escape_line(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\r', "").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Control {
    pub fn to_json(&self) -> String {
        match self {
            Control::Patch { rev } => format!("{{\"kind\":\"patch\",\"rev\":{rev}}}"),
            Control::Module { rev } => format!("{{\"kind\":\"module\",\"rev\":{rev}}}"),
            Control::Reload { rev } => format!("{{\"kind\":\"reload\",\"rev\":{rev}}}"),
            Control::Css { rev, href } => {
                format!("{{\"kind\":\"css\",\"rev\":{rev},\"href\":{}}}", json_string(href))
            }
            Control::Error { rev, text } => {
                format!("{{\"kind\":\"error\",\"rev\":{rev},\"text\":{}}}", json_string(text))
            }
            Control::NoSsr { rev } => format!("{{\"kind\":\"nossr\",\"rev\":{rev}}}"),
            Control::Data { rev, .. } => format!("{{\"kind\":\"data\",\"rev\":{rev}}}"),
            Control::Progress { rev, done, total, label } => {
                format!(
                    "{{\"kind\":\"progress\",\"rev\":{rev},\"done\":{done},\"total\":{total},\"label\":{}}}",
                    json_string(label)
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_round_trip() {
        assert_eq!(parse_control("reload 3\n"), Some(Control::Reload { rev: 3 }));
        assert_eq!(
            parse_control("css 4 /assets/a b.css"),
            Some(Control::Css { rev: 4, href: "/assets/a b.css".into() })
        );
        let msg = "line one\nback\\slash";
        assert_eq!(
            parse_control(&format!("error 5 {}", escape_line(msg))),
            Some(Control::Error { rev: 5, text: msg.into() })
        );
        assert_eq!(
            parse_control("progress 6 12 80 serde, tokio"),
            Some(Control::Progress { rev: 6, done: 12, total: 80, label: "serde, tokio".into() })
        );
        assert_eq!(
            parse_control("progress 6 0 0 "),
            Some(Control::Progress { rev: 6, done: 0, total: 0, label: String::new() })
        );
        assert_eq!(parse_control("progress 6 x 0 a"), None);
        assert!(
            Control::Progress { rev: 1, done: 2, total: 3, label: "a".into() }
                .to_json()
                .contains("\"total\":3")
        );
        assert_eq!(parse_control("patch 9"), Some(Control::Patch { rev: 9 }));
        assert_eq!(parse_control("module 5"), Some(Control::Module { rev: 5 }));
        assert_eq!(Control::Module { rev: 5 }.to_json(), "{\"kind\":\"module\",\"rev\":5}");
        assert_eq!(Control::Patch { rev: 9 }.to_json(), "{\"kind\":\"patch\",\"rev\":9}");
        assert_eq!(parse_control("nossr 9"), Some(Control::NoSsr { rev: 9 }));
        assert_eq!(
            parse_control("data 2 /a b/c.ftl"),
            Some(Control::Data { rev: 2, path: "/a b/c.ftl".into() })
        );
        assert_eq!(parse_control("data 2"), None);
        assert_eq!(parse_control("bogus 1"), None);
        assert_eq!(parse_control("reload x"), None);
        assert_eq!(parse_control(""), None);
        assert!(Control::Css { rev: 1, href: "/a\"b".into() }.to_json().contains("\\\"b"));
    }
}
