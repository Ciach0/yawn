use std::ops::Range;

/// A compile error in an `.rsx` file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    /// Byte range in the source the error points at
    pub span: Range<usize>,
    /// Short text printed under the carets
    pub label: Option<String>,
    /// Secondary locations, e.g. "opening tag here"
    pub notes: Vec<(Range<usize>, String)>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Range<usize>) -> Self {
        let span = if span.end < span.start { span.start..span.start } else { span };
        Diagnostic { message: message.into(), span, label: None, notes: Vec::new(), help: None }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn note(mut self, span: Range<usize>, message: impl Into<String>) -> Self {
        self.notes.push((span, message.into()));
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// 1-based line and column of the start of the span
    pub fn position(&self, source: &str) -> (usize, usize) {
        LineIndex::new(source).position(source, self.span.start)
    }

    /// Multi-line, human-readable form:
    ///
    /// ```text
    /// error: mismatched closing tag
    ///   --> src/counter.rsx:12:5
    ///    |
    /// 12 |     </span>
    ///    |     ^^^^^^^ expected `</div>`
    /// ```
    pub fn render(&self, path: &str, source: &str) -> String {
        let index = LineIndex::new(source);
        let mut out = format!("error: {}\n", self.message);
        let (line, col) = index.position(source, self.span.start);
        let mut spans: Vec<(&Range<usize>, Option<&str>, bool)> =
            vec![(&self.span, self.label.as_deref(), true)];
        for (s, m) in &self.notes {
            spans.push((s, Some(m.as_str()), false));
        }
        let width = spans
            .iter()
            .map(|(s, _, _)| index.position(source, s.start).0.to_string().len())
            .max()
            .unwrap_or(1);
        out.push_str(&format!("{:width$}--> {path}:{line}:{col}\n", "", width = width + 1));
        out.push_str(&format!("{:width$} |\n", "", width = width));
        for (i, (span, label, primary)) in spans.iter().enumerate() {
            let (l, c) = index.position(source, span.start);
            let text = index.line_text(source, l);
            let shown = text.trim_end();
            // Underline up to the end of the first line of the span.
            let line_end_col = shown.chars().count() + 1;
            let end_col = if index.position(source, span.end.max(span.start)).0 == l {
                index.position(source, span.end.max(span.start)).1
            } else {
                line_end_col
            };
            let len = end_col.saturating_sub(c).max(1);
            let mark = if *primary { '^' } else { '-' };
            out.push_str(&format!("{l:>width$} | {shown}\n"));
            out.push_str(&format!(
                "{:width$} | {}{}{}\n",
                "",
                " ".repeat(c - 1),
                mark.to_string().repeat(len),
                label.map(|t| format!(" {t}")).unwrap_or_default(),
            ));
            if i + 1 < spans.len() {
                out.push_str(&format!("{:width$} |\n", "", width = width));
            }
        }
        if let Some(help) = &self.help {
            out.push_str(&format!("{:width$} = help: {help}\n", "", width = width));
        }
        out
    }
}

/// Byte offset -> line/column lookup
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(source.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
        LineIndex { starts }
    }

    /// 1-based (line, column in chars)
    pub fn position(&self, source: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(source.len());
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let mut off = offset;
        while !source.is_char_boundary(off) {
            off -= 1;
        }
        let col = source[self.starts[line]..off].chars().count();
        (line + 1, col + 1)
    }

    /// Text of a 1-based line without its newline
    pub fn line_text<'a>(&self, source: &'a str, line: usize) -> &'a str {
        let start = self.starts[line - 1];
        let end = self.starts.get(line).copied().unwrap_or(source.len());
        source[start..end].trim_end_matches(['\n', '\r'])
    }

    /// Byte offset of a 0-based line and char column
    pub fn offset(&self, source: &str, line0: usize, col0: usize) -> usize {
        let start = self.starts.get(line0).copied().unwrap_or(source.len());
        let rest = &source[start..];
        let line_len = rest.find('\n').unwrap_or(rest.len());
        let line = &rest[..line_len];
        start + line.char_indices().nth(col0).map(|(i, _)| i).unwrap_or(line.len())
    }
}
