use crate::symbols::components;
use std::io::Write;
use std::process::{Command, Stdio};

/// Replacements `(start, end, new text)` as byte ranges in `src`
pub fn format(src: &str) -> Vec<(usize, usize, String)> {
    let comps = components(src);
    let mut edits = Vec::new();
    let mut gaps = Vec::new();
    let mut prev = 0;
    for c in &comps {
        gaps.push((prev, c.full.0));
        prev = c.full.1;
    }
    gaps.push((prev, src.len()));
    for (a, b) in gaps {
        if let Some(e) = fmt_span(src, a, b, rustfmt) {
            edits.push(e);
        }
    }
    for c in &comps {
        let body = |t: &str| {
            let wrapped = format!("fn __yawn() {{\n{t}\n}}\n");
            let out = rustfmt(&wrapped)?;
            let lines: Vec<&str> = out.lines().collect();
            // `fn __yawn() {` ... `}`: the body is already indented by four spaces
            if lines.len() < 2 || !lines[0].starts_with("fn __yawn()") || lines.last() != Some(&"}")
            {
                return None;
            }
            Some(lines[1..lines.len() - 1].join("\n"))
        };
        if let Some(e) = fmt_span(src, c.setup.0, c.setup.1, body) {
            edits.push(e);
        }
    }
    edits.sort_by_key(|e| e.0);
    edits
}

fn fmt_span(
    src: &str,
    a: usize,
    b: usize,
    f: impl Fn(&str) -> Option<String>,
) -> Option<(usize, usize, String)> {
    let text = &src[a..b];
    let lead = text.len() - text.trim_start().len();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let out = f(trimmed)?;
    let out = out.trim_start().trim_end().to_string();
    (out != trimmed).then_some((a + lead, a + lead + trimmed.len(), out))
}

fn rustfmt(text: &str) -> Option<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2021"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(text.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| String::from_utf8(out.stdout).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(src: &str) -> String {
        let mut out = src.to_string();
        for (a, b, t) in format(src).into_iter().rev() {
            out.replace_range(a..b, &t);
        }
        out
    }

    #[test]
    fn formats_setup_and_items_but_not_markup() {
        if rustfmt("fn a(){}").is_none() {
            return; // no rustfmt on this machine
        }
        let src = "use  a::{b,c};\ncomponent A() {\n    let x   =  1 ;\n    let y=x+1;\n    view {\n        <p   class=\"a\">{x}</p>\n    }\n}\n";
        let out = apply(src);
        assert!(out.contains("use a::{b, c};"), "{out}");
        assert!(out.contains("    let x = 1;\n    let y = x + 1;\n    view {"), "{out}");
        assert!(out.contains("<p   class=\"a\">{x}</p>"), "{out}");
    }

    #[test]
    fn unparseable_code_is_left_alone() {
        assert!(apply("component A() {\n    let x =\n    view { <b/> }\n}\n").contains("let x ="));
    }
}
