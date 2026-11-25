use std::cell::Cell;
use std::rc::Rc;
use yawn::browser::global::{Target, listen_global};

/// A finished swipe: where the finger went down (`x`, `y` in the viewport) and how far it travelled
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swipe {
    pub x: f64,
    pub y: f64,
    pub dx: f64,
    pub dy: f64,
}

#[cfg(target_arch = "wasm32")]
mod clock {
    use wasm_bindgen::prelude::wasm_bindgen;
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = Date, js_name = now)]
        pub fn now() -> f64;
    }
}

fn now() -> f64 {
    #[cfg(target_arch = "wasm32")]
    return clock::now();
    #[cfg(not(target_arch = "wasm32"))]
    0.0
}

/// Calls `handler` for every mostly horizontal swipe (at least 50 px, quick, and clearly more
/// sideways than up or down) made anywhere on the page, until the owner (the component) is disposed
pub fn on_swipe(mut handler: impl FnMut(Swipe) + 'static) {
    // where and when the finger went down
    let down = Rc::new(Cell::new(None::<(f64, f64, f64)>));
    {
        let down = down.clone();
        listen_global(Target::Document, "touchstart", false, move |ev| {
            // a second finger makes it a pinch
            let one = ev.number(&["touches", "length"]) == Some(1.0);
            let at = ev
                .number(&["touches", "0", "clientX"])
                .zip(ev.number(&["touches", "0", "clientY"]));
            down.set(if one { at.map(|(x, y)| (x, y, now())) } else { None });
        });
    }
    {
        let down = down.clone();
        listen_global(Target::Document, "touchend", false, move |ev| {
            let Some((x, y, at)) = down.take() else {
                return;
            };
            let Some((ex, ey)) = ev.number(&["changedTouches", "0", "clientX"]).zip(ev.number(&[
                "changedTouches",
                "0",
                "clientY",
            ])) else {
                return;
            };
            let (dx, dy) = (ex - x, ey - y);
            if dx.abs() >= 50.0 && dx.abs() > dy.abs() * 1.5 && now() - at < 700.0 {
                handler(Swipe { x, y, dx, dy });
            }
        });
    }
    listen_global(Target::Document, "touchcancel", false, move |_| down.set(None));
}
