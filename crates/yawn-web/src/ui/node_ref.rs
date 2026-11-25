use super::backend::{NodeId, Rect};
use super::rt::with_backend;
use super::signals::{Sig, signal};

/// An element of the page: where it is, keyboard focus, scrolling
///
/// Get one from a [`NodeRef`] (`ref={...}` on the element) or, for an element you don't hold a
/// handle to, from [`element`] by its DOM `id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Element(NodeId);

/// The element with this DOM `id` as it is now, or `None` when the page has none
pub fn element(id: &str) -> Option<Element> {
    with_backend(|b| b.element_by_id(id)).map(Element)
}

impl Element {
    /// Where the element is on screen now. `None` when there's no layout (on the server)
    pub fn rect(self) -> Option<Rect> {
        with_backend(|b| b.rect(self.0))
    }

    /// Moves keyboard focus to the element
    pub fn focus(self) {
        with_backend(|b| b.focus(self.0, true));
    }

    /// Takes keyboard focus away from the element
    pub fn blur(self) {
        with_backend(|b| b.focus(self.0, false));
    }

    /// Scrolls just enough to show the element
    pub fn scroll_into_view(self) {
        with_backend(|b| b.scroll_into_view(self.0));
    }

    /// The browser object of the element, for what `yawn` doesn't wrap: convert it with
    /// `unchecked_into::<web_sys::Element>()`
    #[cfg(target_arch = "wasm32")]
    pub fn js_value(self) -> Option<wasm_bindgen::JsValue> {
        with_backend(|b| b.js_value(self.0))
    }
}

/// A handle to one element of the page, filled in by `ref={...}` on the element
///
/// It's empty until the element exists and empty again once the element is gone, so code that
/// reads it in an effect follows the element's life:
///
/// ```ignore
/// let input = node_ref();
/// effect(move || {
///     if input.get().is_some() {
///         input.focus();
///     }
/// });
/// view { <input ref={input} /> }
/// ```
#[derive(Clone, Copy)]
pub struct NodeRef(Sig<Option<NodeId>>);

/// An empty [`NodeRef`]
pub fn node_ref() -> NodeRef {
    NodeRef(signal(None))
}

impl NodeRef {
    /// The element, or `None` before it's built and after it's gone; inside an effect or memo this
    /// subscribes to its coming and going
    pub fn get(self) -> Option<Element> {
        self.0.get().map(Element)
    }

    pub(crate) fn set(self, node: NodeId) {
        self.0.set(Some(node));
    }

    pub(crate) fn clear(self) {
        self.0.try_update(|v| *v = None);
    }

    fn element(self) -> Option<Element> {
        self.0.peek().map(Element)
    }

    /// Where the element is on screen now; `None` when there's no element or no layout
    pub fn rect(self) -> Option<Rect> {
        self.element()?.rect()
    }

    /// Moves keyboard focus to the element
    pub fn focus(self) {
        if let Some(e) = self.element() {
            e.focus();
        }
    }

    /// Takes keyboard focus away from the element
    pub fn blur(self) {
        if let Some(e) = self.element() {
            e.blur();
        }
    }

    /// Scrolls just enough to show the element
    pub fn scroll_into_view(self) {
        if let Some(e) = self.element() {
            e.scroll_into_view();
        }
    }

    /// The browser object of the element; see [`Element::js_value`]
    #[cfg(target_arch = "wasm32")]
    pub fn js_value(self) -> Option<wasm_bindgen::JsValue> {
        self.element()?.js_value()
    }
}
