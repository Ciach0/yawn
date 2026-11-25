use lsp_types::Position;
use yawn_rsx::SourceMap;

pub struct Text {
    pub s: String,
    starts: Vec<usize>,
}

impl Text {
    pub fn new(s: String) -> Text {
        let mut starts = vec![0];
        starts.extend(s.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
        Text { s, starts }
    }

    pub fn line(&self, line: usize) -> &str {
        let Some(&start) = self.starts.get(line) else {
            return "";
        };
        let end = self.starts.get(line + 1).copied().unwrap_or(self.s.len());
        self.s[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn offset(&self, p: Position) -> usize {
        let Some(&start) = self.starts.get(p.line as usize) else {
            return self.s.len();
        };
        let line = self.line(p.line as usize);
        let mut units = 0;
        for (i, c) in line.char_indices() {
            if units >= p.character {
                return start + i;
            }
            units += c.len_utf16() as u32;
        }
        start + line.len()
    }

    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.s.len());
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let col: usize = self.s[self.starts[line]..offset].chars().map(char::len_utf16).sum();
        Position::new(line as u32, col as u32)
    }

    fn char_col(&self, p: Position) -> u32 {
        let mut units = 0;
        for (n, c) in self.line(p.line as usize).chars().enumerate() {
            if units >= p.character {
                return n as u32;
            }
            units += c.len_utf16() as u32;
        }
        self.line(p.line as usize).chars().count() as u32
    }

    fn utf16_col(&self, line: u32, char_col: u32) -> u32 {
        self.line(line as usize).chars().take(char_col as usize).map(|c| c.len_utf16() as u32).sum()
    }
}

/// Generated file position -> `.rsx` position, and whether it's inside copied Rust text
pub fn gen_to_src(
    map: &SourceMap,
    generated: &Text,
    src: &Text,
    p: Position,
) -> Option<(Position, bool)> {
    let (l, c, exact) = map.map_exact(p.line + 1, generated.char_col(p) + 1)?;
    let (line0, col0) = (l - 1, c - 1);
    Some((Position::new(line0, src.utf16_col(line0, col0)), exact))
}

/// `.rsx` position -> generated file position, for positions inside copied Rust text
pub fn src_to_gen(map: &SourceMap, src: &Text, generated: &Text, p: Position) -> Option<Position> {
    let (l, c) = map.unmap(p.line + 1, src.char_col(p) + 1)?;
    let (line0, col0) = (l - 1, c - 1);
    Some(Position::new(line0, generated.utf16_col(line0, col0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_round_trip() {
        let t = Text::new("a😀b\nxy".into());
        assert_eq!(t.position(t.offset(Position::new(0, 3))), Position::new(0, 3));
        assert_eq!(t.offset(Position::new(1, 1)), "a😀b\nx".len());
        assert_eq!(t.char_col(Position::new(0, 3)), 2);
    }

    #[test]
    fn maps_both_ways() {
        let src = "component A() {\n    let n = signal(0);\n    view { <p>{n}</p> }\n}\n";
        let c = yawn_rsx::compile(src, "t.rsx").unwrap();
        let (s, g) = (Text::new(src.into()), Text::new(c.rust.clone()));
        let at = Position::new(1, 8); // `n` in `let n`
        let gp = src_to_gen(&c.map, &s, &g, at).unwrap();
        assert!(g.line(gp.line as usize).contains("let n = signal(0)"));
        assert_eq!(gen_to_src(&c.map, &g, &s, gp), Some((at, true)));
    }
}
