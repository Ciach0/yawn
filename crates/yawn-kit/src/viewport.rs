use yawn::browser::global::{Target, listen_global};
use yawn::ui::{Sig, signal};

/// The size of the window's viewport in CSS pixels; `0` by `0` on the server
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
}

/// The size of the viewport now, for a handler or a measurement; `0` by `0` on the server. Use
/// [`viewport`] where something should follow the window being resized.
pub fn size() -> Viewport {
    #[cfg(target_arch = "wasm32")]
    return Viewport {
        width: crate::glue::page::viewport_width(),
        height: crate::glue::page::viewport_height(),
    };
    #[cfg(not(target_arch = "wasm32"))]
    Viewport::default()
}

/// The size of the viewport as a signal that follows the window being resized, until the owner (the
/// component) is disposed
///
/// On the server it stays `0` by `0`, so don't print it into markup the browser will adopt as is.
pub fn viewport() -> Sig<Viewport> {
    let current = signal(size());
    listen_global(Target::Window, "resize", false, move |_| current.set(size()));
    current
}

/// Whether a CSS media query matches now, for a handler or a measurement; `false` on the server.
/// Use [`media`] where something should follow it.
pub fn matches(query: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    return crate::glue::page::matches_media(query);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = query;
        false
    }
}

/// Whether a CSS media query matches, as a signal that follows it, until the owner (the component)
/// is disposed: `media("(pointer: coarse)")` is true on a phone or tablet,
/// `media("(prefers-color-scheme: dark)")` follows the system theme. Always `false` on the server.
pub fn media(query: &'static str) -> Sig<bool> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;
        let on = signal(crate::glue::page::matches_media(query));
        let listener = Closure::<dyn FnMut(bool)>::new(move |matches: bool| {
            yawn::ui::run(|| on.set(matches));
        });
        let id = crate::glue::page::watch_media(query, listener.as_ref().unchecked_ref());
        yawn::ui::on_cleanup(move || {
            crate::glue::page::unwatch_media(id);
            drop(listener);
        });
        on
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = query;
        signal(false)
    }
}
