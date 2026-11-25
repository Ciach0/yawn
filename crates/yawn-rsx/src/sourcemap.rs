/// Where one generated line came from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineOrigin {
    pub src_line: u32,
    pub src_col: u32,
    /// Generated column at which `src_col` starts (only meaningful when `exact`)
    pub gen_col: u32,
    /// The generated text from `gen_col` on is a verbatim copy of source text, so columns map one
    /// to one
    pub exact: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    /// Path of the `.rsx` file, relative to the project root
    pub source: String,
    pub lines: Vec<LineOrigin>,
}

impl SourceMap {
    /// Translates a position in the generated file to the `.rsx` file
    pub fn map(&self, line: u32, col: u32) -> Option<(u32, u32)> {
        let o = self.lines.get(line.checked_sub(1)? as usize)?;
        Some(if o.exact && col >= o.gen_col {
            (o.src_line, o.src_col + (col - o.gen_col))
        } else {
            (o.src_line, o.src_col)
        })
    }

    /// Like [`SourceMap::map`], also saying whether the position is inside copied source text
    pub fn map_exact(&self, line: u32, col: u32) -> Option<(u32, u32, bool)> {
        let o = self.lines.get(line.checked_sub(1)? as usize)?;
        Some(if o.exact && col >= o.gen_col {
            (o.src_line, o.src_col + (col - o.gen_col), true)
        } else {
            (o.src_line, o.src_col, false)
        })
    }

    /// The reverse: a position in the `.rsx` file to the generated file, if that text was copied
    /// verbatim (Rust code)
    pub fn unmap(&self, line: u32, col: u32) -> Option<(u32, u32)> {
        let mut best: Option<(u32, u32, u32)> = None; // (src_col, gen_line, gen_col)
        for (i, o) in self.lines.iter().enumerate() {
            if o.exact
                && o.src_line == line
                && o.src_col <= col
                && best.is_none_or(|b| o.src_col >= b.0)
            {
                best = Some((o.src_col, i as u32 + 1, o.gen_col));
            }
        }
        let (src_col, gen_line, gen_col) = best?;
        Some((gen_line, gen_col + (col - src_col)))
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("yawn-rsx-map 1\nsource {}\n", self.source);
        for l in &self.lines {
            out.push_str(&format!(
                "{} {} {} {}\n",
                l.src_line, l.src_col, l.gen_col, l.exact as u8
            ));
        }
        out
    }

    pub fn parse(text: &str) -> Option<SourceMap> {
        let mut it = text.lines();
        if it.next()? != "yawn-rsx-map 1" {
            return None;
        }
        let source = it.next()?.strip_prefix("source ")?.to_string();
        let mut lines = Vec::new();
        for l in it {
            let mut f = l.split(' ').map(|x| x.parse::<u32>());
            lines.push(LineOrigin {
                src_line: f.next()?.ok()?,
                src_col: f.next()?.ok()?,
                gen_col: f.next()?.ok()?,
                exact: f.next()?.ok()? == 1,
            });
        }
        Some(SourceMap { source, lines })
    }
}
