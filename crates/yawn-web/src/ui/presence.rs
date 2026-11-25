use crate::ui::signals::Sig;

/// Follows `open`, but stays `true` for `ms` milliseconds after `open` turns `false`, so a list can
/// fade out before it is removed
pub fn presence(open: Sig<bool>, ms: i32) -> Sig<bool> {
    #[cfg(target_arch = "wasm32")]
    {
        browser::presence(open, ms)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ms;
        open
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use crate::dom;
    use crate::ui::ambient;
    use crate::ui::signals::{Sig, batch, effect, on_cleanup, signal};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;

    pub fn presence(open: Sig<bool>, ms: i32) -> Sig<bool> {
        let shown = signal(open.get());
        let pending: Rc<Cell<Option<i32>>> = Rc::new(Cell::new(None));
        let keep: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
        {
            let (pending, keep) = (pending.clone(), keep.clone());
            effect(move || {
                let now = open.get();
                if let Some(h) = pending.take() {
                    dom::cancel_timeout(h);
                }
                if now {
                    shown.set(true);
                } else if shown.peek() {
                    let (pending2, shown2) = (pending.clone(), shown);
                    let cb = Closure::<dyn FnMut()>::new(move || {
                        pending2.set(None);
                        ambient::run_if_mounted(|| batch(|| shown2.set(false)));
                    });
                    pending.set(Some(dom::after_ms(&cb, ms)));
                    *keep.borrow_mut() = Some(cb);
                }
            });
        }
        on_cleanup(move || {
            if let Some(h) = pending.take() {
                dom::cancel_timeout(h);
            }
        });
        shown
    }
}
