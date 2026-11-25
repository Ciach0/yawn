use crate::position::Text;
use lsp_types::{Position, Range, TextEdit};
use yawn_rsx::parser::VOID_TAGS;

/// `ch` is the character just typed, `offset` the byte offset after it
pub fn on_type(text: &Text, offset: usize, ch: &str, indent_unit: &str) -> Vec<TextEdit> {
    let src = &text.s;
    match ch {
        ">" => close_tag(src, offset)
            .map(|name| vec![TextEdit { range: at(text, offset), new_text: format!("</{name}>") }])
            .unwrap_or_default(),
        "\n" => split_pair(text, offset, indent_unit),
        _ => Vec::new(),
    }
}

fn at(text: &Text, offset: usize) -> Range {
    let p = text.position(offset);
    Range::new(p, p)
}

fn close_tag(src: &str, offset: usize) -> Option<String> {
    let head = src.get(..offset)?;
    let tag = head.strip_suffix('>')?;
    if tag.ends_with('/') || tag.ends_with('-') || tag.ends_with('=') {
        return None;
    }
    let line_start = tag.rfind('\n').map_or(0, |n| n + 1);
    let lt = line_start + tag[line_start..].rfind('<')?;
    let inner = &tag[lt + 1..];
    let name: String =
        inner.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-')).collect();
    if name.is_empty() || inner.starts_with('/') || !name.starts_with(|c: char| c.is_alphabetic()) {
        return None;
    }
    if name == "slot" || VOID_TAGS.contains(&name.as_str()) {
        return None;
    }
    // `<` must open a tag, not be an operator: it sits at the start of the line or after a delimiter
    if !tag[line_start..lt].trim_end().is_empty()
        && !tag[line_start..lt]
            .ends_with(|c: char| c.is_whitespace() || c == '>' || c == '{' || c == '}')
    {
        return None;
    }
    if src[offset..].starts_with(&format!("</{name}")) {
        return None;
    }
    Some(name)
}

fn split_pair(text: &Text, offset: usize, unit: &str) -> Vec<TextEdit> {
    let src = &text.s;
    let line_start = src[..offset].rfind('\n').map_or(0, |n| n + 1);
    let after = &src[offset..];
    if !after.starts_with("</") {
        return Vec::new();
    }
    // the previous line ends with the matching start tag
    let Some(prev_end) = line_start.checked_sub(1) else {
        return Vec::new();
    };
    let prev_start = src[..prev_end].rfind('\n').map_or(0, |n| n + 1);
    let prev = src[prev_start..prev_end].trim_end();
    let Some(name) = close_tag(src, prev_start + prev.len()) else {
        return Vec::new();
    };
    if !after.starts_with(&format!("</{name}")) {
        return Vec::new();
    }
    let base: String = src[prev_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let cursor_line_indent = src[line_start..offset].len();
    let inner = format!("{base}{unit}");
    let start = text.position(line_start);
    let cur = text.position(offset);
    // replace the (auto-)indent before the cursor, then put `</x>` on its own line after the cursor
    vec![
        TextEdit {
            range: Range::new(
                start,
                Position::new(
                    cur.line,
                    start.character + text.position(line_start + cursor_line_indent).character,
                ),
            ),
            new_text: inner,
        },
        TextEdit { range: Range::new(cur, cur), new_text: format!("\n{base}") },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(src: &str, ch: &str) -> Vec<String> {
        let off = src.find('|').unwrap();
        let t = Text::new(src.replace('|', ""));
        on_type(&t, off, ch, "    ").into_iter().map(|e| e.new_text).collect()
    }

    #[test]
    fn closes_start_tags_only() {
        assert_eq!(typed("component A() { view {\n    <style>|\n} }", ">"), vec!["</style>"]);
        assert_eq!(
            typed("component A() { view {\n    <Card title={t}>|\n} }", ">"),
            vec!["</Card>"]
        );
        assert!(typed("component A() { view {\n    <br>|\n} }", ">").is_empty());
        assert!(typed("component A() { view {\n    <b />|\n} }", ">").is_empty());
        assert!(typed("component A() { view {\n    <b>|</b>\n} }", ">").is_empty());
        assert!(typed("let a = x->|;", ">").is_empty());
    }

    #[test]
    fn enter_between_tags_opens_a_line() {
        let e = typed("component A() { view {\n    <div>\n    |</div>\n} }", "\n");
        assert_eq!(e, vec!["        ".to_string(), "\n    ".to_string()]);
    }
}
