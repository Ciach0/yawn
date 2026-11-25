/// Where a global listener sits
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Document,
    Window,
}

/// A browser event, as far as a handler may read it: any number, string or flag on it or on what it
/// holds, by path
pub struct RawEvent {
    #[cfg(target_arch = "wasm32")]
    event: crate::dom::Event,
}

impl RawEvent {
    /// A number at `path` (`["clientX"]`, `["touches", "0", "clientY"]`), or `None` if it isn't
    /// there
    pub fn number(&self, path: &[&str]) -> Option<f64> {
        #[cfg(target_arch = "wasm32")]
        return self.at(path).and_then(|v| v.as_f64());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = path;
            None
        }
    }

    /// A string at `path` (`["key"]`, `["pointerType"]`), or `None`
    pub fn string(&self, path: &[&str]) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        return self.at(path).and_then(|v| v.as_string());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = path;
            None
        }
    }

    /// A flag at `path` (`["ctrlKey"]`); `false` when it isn't there
    pub fn flag(&self, path: &[&str]) -> bool {
        #[cfg(target_arch = "wasm32")]
        return self.at(path).and_then(|v| v.as_bool()).unwrap_or(false);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = path;
            false
        }
    }

    /// Whether the event happened on or inside the element with this `id`
    pub fn inside(&self, id: &str) -> bool {
        #[cfg(target_arch = "wasm32")]
        return crate::dom::document()
            .and_then(|d| d.get_element_by_id(id))
            .zip(self.event.target())
            .is_some_and(|(el, t)| el.contains(Some(&t)));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = id;
            false
        }
    }

    /// Whether the event happened on or inside an element that matches the CSS `selector`
    /// (`[data-zone]`, `.card`)
    pub fn within(&self, selector: &str) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            self.event.target().and_then(|t| t.closest(selector).ok().flatten()).is_some()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = selector;
            false
        }
    }

    /// Whether the event happened on or inside the element with this `id`, or on something that is
    /// no longer in the page
    pub fn inside_or_gone(&self, id: &str) -> bool {
        #[cfg(target_arch = "wasm32")]
        return crate::dom::document()
            .and_then(|d| d.get_element_by_id(id))
            .zip(self.event.target())
            .is_some_and(|(el, t)| !t.is_connected() || el.contains(Some(&t)));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = id;
            false
        }
    }

    /// Keeps the browser from doing its own thing with this event (scrolling, the text box's undo)
    pub fn prevent_default(&self) {
        #[cfg(target_arch = "wasm32")]
        self.event.prevent_default();
    }

    /// Keeps the event from reaching the handlers of the page
    pub fn stop_propagation(&self) {
        #[cfg(target_arch = "wasm32")]
        self.event.stop_propagation();
    }

    #[cfg(target_arch = "wasm32")]
    fn at(&self, path: &[&str]) -> Option<wasm_bindgen::JsValue> {
        let (first, rest) = path.split_first()?;
        let mut value = crate::dom::reflect_get(&self.event, first).ok()?;
        for key in rest {
            if value.is_undefined() || value.is_null() {
                return None;
            }
            value = crate::dom::reflect_get(&value, key).ok()?;
        }
        if value.is_undefined() || value.is_null() { None } else { Some(value) }
    }
}

/// Calls `handler` for every `kind` event on the document or window, in the capture phase if
/// `capture`, until the component that called this is disposed
#[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
pub fn listen_global(
    target: Target,
    kind: &'static str,
    capture: bool,
    mut handler: impl FnMut(&RawEvent) + 'static,
) {
    #[cfg(target_arch = "wasm32")]
    {
        use crate::ui::signals::{batch, on_cleanup};
        use wasm_bindgen::closure::Closure;
        let (document, window) = (crate::dom::document(), crate::dom::window());
        let node = match target {
            Target::Document => document.as_ref().map(|d| d.target()),
            Target::Window => window.as_ref().map(|w| w.target()),
        };
        let Some(node) = node else { return };
        let owner = crate::ui::ambient::capture_owner();
        let listener =
            Closure::<dyn FnMut(crate::dom::Event)>::new(move |event: crate::dom::Event| {
                let raw = RawEvent { event };
                // the listener runs outside the runtime
                crate::ui::ambient::run_in_owner(owner, || batch(|| handler(&raw)));
            });
        let _ = node.add_event_listener_capture(kind, &listener, capture);
        on_cleanup(move || {
            let (document, window) = (crate::dom::document(), crate::dom::window());
            let node = match target {
                Target::Document => document.as_ref().map(|d| d.target()),
                Target::Window => window.as_ref().map(|w| w.target()),
            };
            if let Some(node) = node {
                let _ = node.remove_event_listener_capture(kind, &listener, capture);
            }
            crate::task::defer_drop(listener);
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (target, kind, capture, handler);
}

/// Stops the next click on the page from reaching anything, and gives up on it after the current
/// task if none comes
pub fn swallow_next_click() {
    #[cfg(target_arch = "wasm32")]
    {
        use std::cell::RefCell;
        use std::rc::Rc;
        use wasm_bindgen::closure::Closure;
        let Some(document) = crate::dom::document() else {
            return;
        };
        let slot: Rc<RefCell<Option<Closure<dyn FnMut(crate::dom::Event)>>>> =
            Rc::new(RefCell::new(None));
        let remove = {
            let (document, slot) = (document.clone(), slot.clone());
            move || {
                if let Some(f) = slot.borrow_mut().take() {
                    let _ = document.target().remove_event_listener_capture("click", &f, true);
                    // this runs inside the closure's own call when the click arrives: it may not be dropped here
                    crate::task::defer_drop(f);
                }
            }
        };
        let on_click = {
            let remove = remove.clone();
            Closure::<dyn FnMut(crate::dom::Event)>::new(move |ev: crate::dom::Event| {
                ev.stop_propagation();
                remove();
            })
        };
        let _ = document.target().add_event_listener_capture("click", &on_click, true);
        *slot.borrow_mut() = Some(on_click);
        let timer = Closure::<dyn FnMut()>::once(remove);
        crate::dom::after_ms(&timer, 0);
        timer.forget();
    }
}
