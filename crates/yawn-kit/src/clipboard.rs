/// Copies `text` to the clipboard
pub fn copy(text: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::glue::page::copy_text(text);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = text;
}
