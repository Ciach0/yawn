#[cfg(feature = "editing")]
pub mod editing {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(module = "/src/js/editing.js")]
    extern "C" {
        #[wasm_bindgen(js_name = boxValue)]
        pub fn box_value(id: &str) -> Option<String>;
        #[wasm_bindgen(js_name = boxSelection)]
        fn box_selection_raw(id: &str) -> Vec<f64>;
        #[wasm_bindgen(js_name = boxSetSelection)]
        pub fn box_set_selection(id: &str, a: u32, b: u32);
        #[wasm_bindgen(js_name = boxInsert)]
        pub fn box_insert(id: &str, text: &str) -> bool;
        #[wasm_bindgen(js_name = boxAutosize)]
        pub fn box_autosize(id: &str);
        #[wasm_bindgen(js_name = boxSetHtml)]
        pub fn box_set_html(id: &str, html: &str, a: i32, b: i32, focus: bool);
        #[wasm_bindgen(js_name = holdTouchScroll)]
        pub fn hold_touch_scroll(on: bool);
        #[wasm_bindgen(js_name = releaseFocus)]
        pub fn release_focus();
        #[wasm_bindgen(js_name = textSelects)]
        pub fn text_selects(x: f64, y: f64) -> bool;
        #[wasm_bindgen(js_name = scrollBox)]
        pub fn scroll_box(id: &str, dy: f64);
        #[wasm_bindgen(js_name = autoScroll)]
        pub fn auto_scroll(x: f64, y: f64, edge: f64, step: f64);
        #[wasm_bindgen(js_name = caretRect)]
        fn caret_rect_raw(id: &str) -> Vec<f64>;
        #[wasm_bindgen(js_name = moveLine)]
        pub fn move_line(id: &str, selector: &str, delta: i32) -> bool;
    }

    /// The selection of a box as UTF-16 units, or `None` when it has none
    pub fn box_selection(id: &str) -> Option<(f64, f64)> {
        match box_selection_raw(id)[..] {
            [a, b] => Some((a, b)),
            _ => None,
        }
    }

    /// Where the caret is in the box with this `id`, `[left, top, right, bottom]` in the viewport;
    /// `None` when the box has no caret
    pub fn caret_rect(id: &str) -> Option<[f64; 4]> {
        match caret_rect_raw(id)[..] {
            [l, t, r, b] => Some([l, t, r, b]),
            _ => None,
        }
    }
}

pub mod page {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(module = "/src/js/page.js")]
    extern "C" {
        #[wasm_bindgen(js_name = keepFocusVisible)]
        pub fn keep_focus_visible();
        #[wasm_bindgen(js_name = viewportWidth)]
        pub fn viewport_width() -> f64;
        #[wasm_bindgen(js_name = viewportHeight)]
        pub fn viewport_height() -> f64;
        #[wasm_bindgen(js_name = copyText)]
        pub fn copy_text(text: &str);
        #[wasm_bindgen(js_name = underPointer)]
        pub fn under_pointer(x: f64, y: f64, attr: &str) -> Option<String>;
        #[wasm_bindgen(js_name = matchesMedia)]
        pub fn matches_media(query: &str) -> bool;
        #[wasm_bindgen(js_name = watchMedia)]
        pub fn watch_media(query: &str, callback: &JsValue) -> i32;
        #[wasm_bindgen(js_name = unwatchMedia)]
        pub fn unwatch_media(id: i32);
    }
}
