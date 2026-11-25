use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// An event as handlers see it
#[derive(Clone, Debug, Default)]
pub struct Event {
    /// `value` of the target for `input`/`change` events, empty otherwise
    pub value: String,
    /// `key` of keyboard events, empty otherwise
    pub key: String,
    /// A mouse event with Ctrl, Meta, Shift or Alt held, or not made with the main button (the
    /// browser would open a new tab or window for a link)
    pub modified: bool,
    /// Modifier keys, one by one (`modified` mixes them with the button)
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    /// Pointer position in the viewport, for mouse, pointer and drag events; `0` for others
    pub x: f64,
    pub y: f64,
    /// `inputType` of an `input` event (`insertText`, `historyUndo`, `insertFromPaste`...), empty
    /// otherwise
    pub input_type: String,
    /// Plain text carried by a `paste` or `drop` event, empty otherwise
    pub text: String,
    /// Files chosen in a file input (`change`) or dropped (`drop`)
    pub files: Vec<FileRef>,
    pub(crate) rect: RectSource,
    pub(crate) prevent: Rc<Cell<bool>>,
    pub(crate) stop: Rc<Cell<bool>>,
}

/// Where an element is on screen, in CSS pixels from the top left of the viewport
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    pub fn width(&self) -> f64 {
        self.right - self.left
    }
    pub fn height(&self) -> f64 {
        self.bottom - self.top
    }
}

/// A file the user picked or dropped
#[derive(Clone)]
pub struct FileRef {
    pub name: String,
    pub size: f64,
    /// Media type the browser reports (`image/png`), possibly empty
    pub mime: String,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) handle: Rc<dyn std::any::Any>,
}

impl std::fmt::Debug for FileRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileRef({} {}B {})", self.name, self.size, self.mime)
    }
}

/// Measures the element a handler is on, when asked (measuring makes the browser lay the page out,
/// so it isn't done for every event)
#[derive(Clone, Default)]
pub struct RectSource(pub(crate) Option<Rc<dyn Fn(Option<&str>) -> Option<[f64; 4]>>>);

impl std::fmt::Debug for RectSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RectSource")
    }
}

impl Event {
    /// Where the element this handler is on is on screen, now
    pub fn target_rect(&self) -> Option<Rect> {
        let [left, top, right, bottom] = (self.rect.0.as_ref()?)(None)?;
        Some(Rect { left, top, right, bottom })
    }

    /// Where the text inside the first descendant of this handler's element that matches `selector`
    /// is, now: the rectangle around the text itself, so for wrapped text its width is that of the
    /// longest line
    pub fn text_rect(&self, selector: &str) -> Option<Rect> {
        let [left, top, right, bottom] = (self.rect.0.as_ref()?)(Some(selector))?;
        Some(Rect { left, top, right, bottom })
    }

    /// Cancels the browser's default action (following a link, submitting a form) once the handler
    /// returns
    pub fn prevent_default(&self) {
        self.prevent.set(true);
    }

    /// Keeps the event from reaching the handlers of the elements around this one
    pub fn stop_propagation(&self) {
        self.stop.set(true);
    }

    /// Whether a handler asked for the default action to be cancelled
    pub fn default_prevented(&self) -> bool {
        self.prevent.get()
    }
}

pub type Handler = Rc<RefCell<dyn FnMut(Event)>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ListenerId(pub u32);

pub trait Backend {
    fn create_element(&mut self, tag: &str) -> NodeId;
    fn create_text(&mut self, text: &str) -> NodeId;
    fn create_comment(&mut self) -> NodeId;
    /// [`create_text`](Self::create_text) for a literal of the source
    fn create_text_lit(&mut self, text: &'static str) -> NodeId {
        self.create_text(text)
    }
    /// A fixed subtree from markup the `.rsx` compiler wrote; returns its root
    fn create_template(&mut self, html: &'static str) -> NodeId {
        super::template::build(self, html)
    }
    /// [`create_template`](Self::create_template) that also hands out nodes inside the subtree:
    /// `out[k]` becomes the node reached from the root by `paths[k]`, a list of child indices (`[]`
    /// is the root)
    fn create_template_holes(
        &mut self,
        html: &'static str,
        paths: &[&'static [u8]],
        out: &mut [NodeId],
    ) {
        super::template::build_holes(self, html, paths, out)
    }
    fn track_literal(&mut self, _kind: &str, _original: &str, _current: &str, _node: NodeId) {}

    fn set_text(&mut self, node: NodeId, text: &str);
    fn set_attr(&mut self, node: NodeId, name: &str, value: &str);
    /// [`set_attr`](Self::set_attr) for a name and value that are literals of the source
    fn set_attr_lit(&mut self, node: NodeId, name: &'static str, value: &'static str) {
        self.set_attr(node, name, value)
    }
    fn remove_attr(&mut self, node: NodeId, name: &str);
    /// DOM property (not attribute), e.g. the `value` of an input
    fn set_prop(&mut self, node: NodeId, name: &str, value: &str);
    fn set_class(&mut self, node: NodeId, class: &str, on: bool);
    fn append(&mut self, parent: NodeId, child: NodeId);
    fn insert_before(&mut self, reference: NodeId, node: NodeId);
    /// Detaches `node` from its parent, if any
    fn remove(&mut self, node: NodeId);
    /// Removes everything between the siblings `start` and `end`, both excluded
    fn remove_between(&mut self, start: NodeId, end: NodeId);
    /// Removes the siblings `start..=end`
    fn remove_range(&mut self, start: NodeId, end: NodeId);
    /// Moves the siblings `start..=end` in front of `reference`
    fn move_range(&mut self, start: NodeId, end: NodeId, reference: NodeId);
    fn listen(&mut self, node: NodeId, event: &str, handler: Handler) -> ListenerId;
    fn unlisten(&mut self, id: ListenerId);
    /// The renderer won't use these ids again; the backend may free its handles
    fn release(&mut self, nodes: &[NodeId]);
    fn set_title(&mut self, title: &str);
    /// Where the element is on screen now; `None` when the backend has no layout (the server, the mock)
    fn rect(&mut self, _node: NodeId) -> Option<Rect> {
        None
    }
    /// The element with this DOM `id` as it is now, or `None` when there is none (or no document)
    fn element_by_id(&mut self, _id: &str) -> Option<NodeId> {
        None
    }
    /// Moves keyboard focus to the element, or takes it away when `on` is false
    fn focus(&mut self, _node: NodeId, _on: bool) {}
    /// Scrolls the nearest scrollable ancestors just enough to show the element
    fn scroll_into_view(&mut self, _node: NodeId) {}
    /// The browser object behind the node, for what `yawn` doesn't wrap
    #[cfg(target_arch = "wasm32")]
    fn js_value(&mut self, _node: NodeId) -> Option<wasm_bindgen::JsValue> {
        None
    }
    /// A tag for the document head (`meta`, `link`): see [`super::head`]
    fn head_tag(&mut self, _tag: &str, _attrs: &[(&str, &str)]) {}
}
