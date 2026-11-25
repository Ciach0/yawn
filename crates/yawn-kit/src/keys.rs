use yawn::browser::global::{Target, listen_global};

/// A key press: `key` as the browser names it (`"z"`, `"Escape"`), and the modifiers
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Calls `handler` for every key pressed anywhere in the page, until the owner (the component) is
/// disposed
pub fn on_key(mut handler: impl FnMut(&KeyPress) -> bool + 'static) {
    listen_global(Target::Document, "keydown", false, move |ev| {
        let press = KeyPress {
            key: ev.string(&["key"]).unwrap_or_default(),
            ctrl: ev.flag(&["ctrlKey"]) || ev.flag(&["metaKey"]),
            shift: ev.flag(&["shiftKey"]),
            alt: ev.flag(&["altKey"]),
        };
        if handler(&press) {
            ev.prevent_default();
        }
    });
}
