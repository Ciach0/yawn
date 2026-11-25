#[cfg(target_arch = "wasm32")]
use crate::glue::editing as glue;

/// The selected part of a text box: `start..end`, equal when it is only a caret
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub start: usize,
    pub end: usize,
}

impl Selection {
    pub fn is_caret(&self) -> bool {
        self.start == self.end
    }
}

/// Byte index in `text` of the UTF-16 unit `unit` (clamped to the end)
pub fn byte_of_utf16(text: &str, unit: usize) -> usize {
    let mut seen = 0;
    for (i, c) in text.char_indices() {
        if seen >= unit {
            return i;
        }
        seen += c.len_utf16();
    }
    text.len()
}

/// UTF-16 unit of the byte index `byte` in `text`
pub fn utf16_of_byte(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().map(char::len_utf16).sum()
}

/// The selection inside the text box (`<input>` or `<textarea>`) with this `id`
pub fn selection(id: &str) -> Option<Selection> {
    #[cfg(target_arch = "wasm32")]
    {
        let value = glue::box_value(id)?;
        let (a, b) = glue::box_selection(id)?;
        Some(Selection {
            start: byte_of_utf16(&value, a as usize),
            end: byte_of_utf16(&value, b as usize),
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
        None
    }
}

/// Selects `start..end` (byte offsets) in the text box with this `id`; a caret when they're equal
pub fn set_selection(id: &str, start: usize, end: usize) {
    #[cfg(target_arch = "wasm32")]
    if let Some(value) = glue::box_value(id) {
        glue::box_set_selection(
            id,
            utf16_of_byte(&value, start) as u32,
            utf16_of_byte(&value, end) as u32,
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (id, start, end);
}

/// Replaces the selection of the text box with `text` and leaves the caret after it
pub fn insert_text(id: &str, text: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    return glue::box_insert(id, text);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (id, text);
        false
    }
}

/// The text of the box with this `id` (a text box, or an element with `contenteditable="true"`
/// holding one `<div>` per line)
pub fn value(id: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    return glue::box_value(id);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
        None
    }
}

/// Replaces what an editable element (`contenteditable="true"`) shows with `html`, and puts the
/// selection `start..end` (byte offsets into `text`, the text that `html` draws) where it was
pub fn set_html(id: &str, html: &str, text: &str, selection: Option<Selection>, focus: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        let (a, b) = selection.map_or((-1, -1), |s| {
            (utf16_of_byte(text, s.start) as i32, utf16_of_byte(text, s.end) as i32)
        });
        glue::box_set_html(id, html, a, b, focus);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (id, html, text, selection, focus);
}

/// Makes a `<textarea>` as tall as its text, so it never scrolls inside itself
pub fn autosize(id: &str) {
    #[cfg(target_arch = "wasm32")]
    glue::box_autosize(id);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}

/// Scrolls the element with this `id` by `dy` pixels (a list that follows a drag to its edge)
pub fn scroll_by(id: &str, dy: f64) {
    #[cfg(target_arch = "wasm32")]
    glue::scroll_box(id, dy);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (id, dy);
}

/// While something is dragged at the viewport point `(x, y)`: scrolls the nearest scrolling
/// ancestor (or the window) by `step` pixels when the point is within `edge` pixels of the top or
/// bottom of the window
pub fn auto_scroll(x: f64, y: f64, edge: f64, step: f64) {
    #[cfg(target_arch = "wasm32")]
    glue::auto_scroll(x, y, edge, step);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (x, y, edge, step);
}

pub use crate::measure::attribute_under;

/// Takes the caret and any selection off the page, so a gesture that's about to move something
/// doesn't also select text
pub fn release_focus() {
    #[cfg(target_arch = "wasm32")]
    glue::release_focus();
}

/// Whether a press at the viewport point `(x, y)` lands on a character of a rich text box that
/// already has the caret, where the browser should select text
pub fn text_selects(x: f64, y: f64) -> bool {
    #[cfg(target_arch = "wasm32")]
    return glue::text_selects(x, y);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (x, y);
        false
    }
}

/// While held, a finger that moves over the page doesn't scroll it
pub fn hold_touch_scroll(on: bool) {
    #[cfg(target_arch = "wasm32")]
    glue::hold_touch_scroll(on);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on;
}

/// Moves the caret from the editable element `id` to the editable text on the line `delta` lines on
/// (negative: back)
pub fn move_line(id: &str, selector: &str, delta: i32) -> bool {
    #[cfg(target_arch = "wasm32")]
    return glue::move_line(id, selector, delta);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (id, selector, delta);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_and_byte_offsets_agree_on_multibyte_text() {
        let s = "aż😀b";
        // 'a' 1 unit, 'ż' 1 unit (2 bytes), '😀' 2 units (4 bytes), 'b'
        assert_eq!(byte_of_utf16(s, 0), 0);
        assert_eq!(byte_of_utf16(s, 2), 3);
        assert_eq!(byte_of_utf16(s, 4), 7);
        assert_eq!(byte_of_utf16(s, 99), s.len());
        assert_eq!(utf16_of_byte(s, 3), 2);
        assert_eq!(utf16_of_byte(s, 7), 4);
        for u in [0, 1, 2, 4, 5] {
            assert_eq!(utf16_of_byte(s, byte_of_utf16(s, u)), u);
        }
    }

    #[test]
    fn nothing_answers_without_a_page() {
        assert_eq!(selection("x"), None);
        assert!(!insert_text("x", "t"));
        assert_eq!(attribute_under(1.0, 1.0, "data-drop"), None);
    }
}
