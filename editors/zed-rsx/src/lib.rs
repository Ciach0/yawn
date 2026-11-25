//! Zed extension: starts `yawn-lsp` for `.rsx` files. Highlighting comes from the grammar and
//! the queries in `languages/rsx`.

use zed::lsp::{Completion, CompletionKind};
use zed::{CodeLabel, CodeLabelSpan};
use zed_extension_api as zed;

struct Rsx;

impl zed::Extension for Rsx {
    fn new() -> Self {
        Rsx
    }

    fn language_server_command(&mut self, _id: &zed::LanguageServerId, worktree: &zed::Worktree) -> zed::Result<zed::Command> {
        let command = worktree
            .which("yawn-lsp")
            .ok_or("`yawn-lsp` is not on PATH. Install it with: cargo install --path crates/yawn-lsp (from the Yawn repository)")?;
        Ok(zed::Command { command, args: Vec::new(), env: worktree.shell_env() })
    }

    /// Rust completions come from rust-analyzer. Without this Zed shows their labels as plain text:
    /// it highlights a label by parsing it with the buffer's grammar, and for `.rsx` that grammar sees
    /// no Rust in a label. So the label is cut into spans with explicit highlight names, the way
    /// Zed's own Rust support colours a signature.
    fn label_for_completion(&self, _id: &zed::LanguageServerId, c: Completion) -> Option<CodeLabel> {
        rust_label(&c)
    }
}

fn rust_label(c: &Completion) -> Option<CodeLabel> {
    use CompletionKind as K;
    // HTML items from yawn-lsp say what they are in `detail`; their kind (Property, Value) would be
    // coloured like a Rust field or a CSS value.
    if let Some(hl) = match c.detail.as_deref() {
        Some("tag") => Some("tag"),
        Some("attribute") => Some("attribute"),
        _ => None,
    } {
        return Some(CodeLabel { code: String::new(), filter_range: (0..c.label.len() as u32).into(), spans: vec![literal(&c.label, Some(hl))] });
    }
    let kind = c.kind?;
    let details = c.label_details.as_ref();
    let name_end = c.label.find(['(', '<', ' ']).unwrap_or(c.label.len());
    let (name, tail) = c.label.split_at(name_end);
    let name_hl = match kind {
        K::Function | K::Method | K::Constructor => "function",
        K::Field | K::Property => "property",
        K::Variable => "variable",
        K::Constant | K::EnumMember => "constant",
        K::Class | K::Struct | K::Enum | K::Interface | K::TypeParameter => "type",
        K::Module => "namespace",
        K::Keyword => "keyword",
        _ => return None,
    };
    let mut spans = vec![literal(name, Some(name_hl))];
    // rust-analyzer sends the signature either in the label or in labelDetails.detail
    let signature = details.and_then(|d| d.detail.as_deref()).filter(|d| !d.is_empty()).unwrap_or(tail);
    push_rust(&mut spans, signature);
    if let Some(desc) = details.and_then(|d| d.description.as_deref()).filter(|d| !d.is_empty() && !d.starts_with("use ")) {
        match kind {
            K::Function | K::Method | K::Constructor => {
                spans.push(literal(" -> ", Some("operator")));
                push_rust(&mut spans, desc);
            }
            _ => {
                spans.push(literal(": ", Some("punctuation.delimiter")));
                push_rust(&mut spans, desc);
            }
        }
    }
    Some(CodeLabel { code: String::new(), filter_range: (0..name.len() as u32).into(), spans })
}

fn literal(text: &str, highlight: Option<&str>) -> CodeLabelSpan {
    CodeLabelSpan::literal(text, highlight.map(str::to_string))
}

/// Splits a Rust signature fragment (`(&mut self, ch: char)`, `Vec<T>`) into highlighted spans.
fn push_rust(out: &mut Vec<CodeLabelSpan>, text: &str) {
    const KEYWORDS: &[&str] = &["mut", "dyn", "impl", "fn", "where", "ref", "const", "unsafe", "async", "move", "for", "in", "as"];
    const PRIMITIVES: &[&str] = &["i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "char", "str"];
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = text[i..].chars().next().unwrap_or(' ');
        let mut j = i + c.len_utf8();
        let hl = if c.is_alphabetic() || c == '_' {
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            let word = &text[i..j];
            let rest = text[j..].trim_start();
            if word == "self" || word == "Self" {
                Some(if word == "self" { "variable.special" } else { "type" })
            } else if KEYWORDS.contains(&word) {
                Some("keyword")
            } else if PRIMITIVES.contains(&word) || word.starts_with(char::is_uppercase) {
                Some("type")
            } else if rest.starts_with("::") {
                Some("namespace")
            } else if rest.starts_with(':') {
                Some("variable")
            } else if rest.starts_with('(') {
                Some("function")
            } else {
                None
            }
        } else if c == '\'' && text[j..].starts_with(|n: char| n.is_alphabetic() || n == '_') {
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            Some("lifetime")
        } else if c.is_ascii_digit() {
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            Some("constant")
        } else {
            match c {
                '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' => Some("punctuation.bracket"),
                ',' | ':' | ';' | '.' => Some("punctuation.delimiter"),
                '&' | '*' | '-' | '+' | '=' | '!' | '?' | '|' => Some("operator"),
                _ => None,
            }
        };
        // `->` and `::` read better as one span
        out.push(literal(&text[i..j], hl));
        i = j;
    }
}

zed::register_extension!(Rsx);
