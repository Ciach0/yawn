pub use wasm_bindgen;

use crate::dom::{self, Node};
use crate::ui::hydrate::{self, Real, RealKind};
use crate::ui::{Backend, Component, Event, Handler, ListenerId, NodeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::intern;

type JsHandler = Closure<dyn FnMut(dom::Event)>;

thread_local! {
    static GRAVEYARD: RefCell<Vec<JsHandler>> = const { RefCell::new(Vec::new()) };
    static SWEEP_PENDING: Cell<bool> = const { Cell::new(false) };
    static SWEEPER: Closure<dyn FnMut()> = Closure::new(|| {
        SWEEP_PENDING.set(false);
        GRAVEYARD.with(|g| g.borrow_mut().clear());
    });
}

fn bury(closure: JsHandler) {
    GRAVEYARD.with(|g| g.borrow_mut().push(closure));
    if !SWEEP_PENDING.replace(true) {
        SWEEPER.with(dom::after_current_call);
    }
}

pub fn document() -> dom::Document {
    dom::document().expect("no document")
}

pub fn log(message: &str) {
    dom::log(message);
}

struct Listener {
    node: Node,
    event: String,
    closure: JsHandler,
}

/// [`Backend`] over the browser DOM
pub struct WebBackend {
    nodes: Vec<Option<Node>>,
    free: Vec<u32>,
    listeners: HashMap<u32, Listener>,
    raw_html: HashMap<u32, String>,
    next_listener: u32,
    templates: HashMap<usize, Node>,
    /// Elements handed out by `element_by_id`, so asking again doesn't take another slot
    by_id: HashMap<String, NodeId>,
    doc: dom::Document,
}

impl WebBackend {
    pub fn new() -> Self {
        WebBackend {
            nodes: Vec::new(),
            free: Vec::new(),
            listeners: HashMap::new(),
            raw_html: HashMap::new(),
            next_listener: 0,
            templates: HashMap::new(),
            by_id: HashMap::new(),
            doc: document(),
        }
    }

    pub fn adopt(&mut self, node: Node) -> NodeId {
        match self.free.pop() {
            Some(i) => {
                self.nodes[i as usize] = Some(node);
                NodeId(i)
            }
            None => {
                self.nodes.push(Some(node));
                NodeId(self.nodes.len() as u32 - 1)
            }
        }
    }

    fn from_table(table: Vec<Option<Node>>, next_listener: u32) -> Self {
        let free =
            table.iter().enumerate().filter(|(_, n)| n.is_none()).map(|(i, _)| i as u32).collect();
        WebBackend {
            nodes: table,
            free,
            listeners: HashMap::new(),
            raw_html: HashMap::new(),
            next_listener,
            templates: HashMap::new(),
            by_id: HashMap::new(),
            doc: document(),
        }
    }

    fn template_copy(&mut self, html: &'static str) -> Node {
        let key = html.as_ptr() as usize;
        if !self.templates.contains_key(&key) {
            let holder = self.doc.create_element("template").expect("create_element");
            holder.set_inner_html(html);
            let root =
                holder.content().first_child().expect("yawn: a template needs a root element");
            self.templates.insert(key, root);
        }
        self.templates[&key].clone_node(true).expect("clone_node")
    }

    fn add_listener(&mut self, id: u32, node: NodeId, event: &str, handler: Handler) {
        let own = self.node(node).clone();
        let rect = crate::ui::RectSource(Some(Rc::new(move |inside: Option<&str>| {
            let r = match inside {
                None => own.get_bounding_client_rect(),
                Some(selector) => {
                    let found = own.query_selector_in(selector).ok().flatten()?;
                    let range = dom::document()?.create_range();
                    range.select_node_contents(&found);
                    range.range_rect()
                }
            };
            let n = |k: &str| dom::get_number(&r, k);
            Some([n("left"), n("top"), n("right"), n("bottom")])
        })));
        // pointer, text and file details cost a few calls into JavaScript: only the events that use them pay
        let wants_extras = matches!(
            event,
            "input"
                | "beforeinput"
                | "paste"
                | "drop"
                | "change"
                | "dragover"
                | "dragenter"
                | "click"
                | "contextmenu"
                | "pointerdown"
                | "pointermove"
                | "pointerup"
                | "mousedown"
                | "mousemove"
                | "mouseup"
        );
        let closure = JsHandler::new(move |ev: dom::Event| {
            // each of these is absent on some kinds of event (`key` on a click), so they're checked
            let flag = |v: wasm_bindgen::JsValue| v.as_bool().unwrap_or(false);
            let modified = flag(ev.ctrl_key())
                || flag(ev.meta_key())
                || flag(ev.shift_key())
                || flag(ev.alt_key())
                || ev.button().as_f64().is_some_and(|b| b != 0.0);
            let value = ev.target().and_then(|t| t.value().as_string()).unwrap_or_default();
            let mut e = Event {
                value,
                key: ev.key().as_string().unwrap_or_default(),
                modified,
                rect: rect.clone(),
                ..Event::default()
            };
            e.ctrl = flag(ev.ctrl_key());
            e.shift = flag(ev.shift_key());
            e.alt = flag(ev.alt_key());
            e.meta = flag(ev.meta_key());
            if wants_extras {
                crate::browser::platform::fill_event(&ev, &mut e);
            }
            let prevented = e.clone();
            if let Ok(mut h) = handler.try_borrow_mut() {
                h(e);
            }
            if prevented.default_prevented() {
                ev.prevent_default();
            }
            if prevented.stop.get() {
                ev.stop_propagation();
            }
        });
        let n = self.node(node).clone();
        let _ = n.target().add_event_listener(intern(event), &closure);
        self.listeners.insert(id, Listener { node: n, event: event.to_string(), closure });
    }

    fn node(&self, id: NodeId) -> &Node {
        self.nodes[id.0 as usize].as_ref().expect("yawn: use of a released node")
    }

    fn live(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)?.as_ref()
    }

    fn element(&self, id: NodeId) -> &Node {
        // The renderer only asks for elements here. The check is a call into JavaScript,
        // so it stays out of release builds.
        let node = self.node(id);
        debug_assert_eq!(node.node_type(), dom::ELEMENT_NODE, "yawn: not an element");
        node
    }
}

impl Default for WebBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for WebBackend {
    fn create_element(&mut self, tag: &str) -> NodeId {
        let el = self.doc.create_element(intern(tag)).expect("create_element");
        self.adopt(el)
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        self.adopt(self.doc.create_text_node(text))
    }

    fn create_template(&mut self, html: &'static str) -> NodeId {
        let copy = self.template_copy(html);
        self.adopt(copy)
    }

    fn create_template_holes(
        &mut self,
        html: &'static str,
        paths: &[&'static [u8]],
        out: &mut [NodeId],
    ) {
        let root = self.template_copy(html);
        // the nodes along the previous path, so that the next path starts where it left off
        let mut chain: Vec<Node> = vec![root];
        let mut last: &[u8] = &[];
        for (slot, path) in out.iter_mut().zip(paths) {
            let common = last.iter().zip(path.iter()).take_while(|(a, b)| a == b).count();
            chain.truncate(common + 1);
            for &index in &path[common..] {
                let parent = chain.last().expect("the root");
                let child =
                    nth_child(parent, index).expect("yawn: a path of a template leads nowhere");
                chain.push(child);
            }
            *slot = self.adopt(chain.last().expect("the node").clone());
            last = path;
        }
    }

    fn create_comment(&mut self) -> NodeId {
        self.adopt(self.doc.create_comment(""))
    }

    fn set_text(&mut self, node: NodeId, text: &str) {
        self.node(node).set_node_value(Some(text));
    }

    fn set_attr(&mut self, node: NodeId, name: &str, value: &str) {
        let _ = self.element(node).set_attribute(intern(name), value);
    }

    fn set_attr_lit(&mut self, node: NodeId, name: &'static str, value: &'static str) {
        let _ = self.element(node).set_attribute(intern(name), intern(value));
    }

    fn create_text_lit(&mut self, text: &'static str) -> NodeId {
        self.adopt(self.doc.create_text_node(intern(text)))
    }

    fn track_literal(&mut self, kind: &str, original: &str, current: &str, node: NodeId) {
        if OVERRIDES.with(|all| all.is_some()) {
            dom::dev_track_literal(kind, original, current, self.node(node));
        }
    }

    fn remove_attr(&mut self, node: NodeId, name: &str) {
        let _ = self.element(node).remove_attribute(intern(name));
    }

    fn set_prop(&mut self, node: NodeId, name: &str, value: &str) {
        if name == "innerHTML" {
            if self.raw_html.get(&node.0).is_some_and(|last| last == value) {
                return;
            }
            self.raw_html.insert(node.0, value.to_string());
        }
        dom::set_string(self.node(node), name, value);
    }

    fn set_class(&mut self, node: NodeId, class: &str, on: bool) {
        let list = self.element(node).class_list();
        let class = intern(class);
        let _ = if on { list.add(class) } else { list.remove(class) };
    }

    fn append(&mut self, parent: NodeId, child: NodeId) {
        let _ = self.node(parent).append_child(self.node(child));
    }

    fn insert_before(&mut self, reference: NodeId, node: NodeId) {
        let r = self.node(reference);
        if let Some(parent) = r.parent_node() {
            let _ = parent.insert_before(self.node(node), Some(r));
        }
    }

    fn remove(&mut self, node: NodeId) {
        let n = self.node(node);
        if let Some(parent) = n.parent_node() {
            let _ = parent.remove_child(n);
        }
    }

    fn remove_between(&mut self, start: NodeId, end: NodeId) {
        let (s, e) = (self.node(start), self.node(end));
        let Some(parent) = s.parent_node() else {
            return;
        };
        let mut cur = s.next_sibling();
        while let Some(n) = cur {
            if &n == e {
                break;
            }
            cur = n.next_sibling();
            let _ = parent.remove_child(&n);
        }
    }

    fn remove_range(&mut self, start: NodeId, end: NodeId) {
        let (s, e) = (self.node(start), self.node(end));
        let Some(parent) = s.parent_node() else {
            return;
        };
        let mut cur = Some(s.clone());
        while let Some(n) = cur {
            let last = &n == e;
            cur = n.next_sibling();
            let _ = parent.remove_child(&n);
            if last {
                break;
            }
        }
    }

    fn move_range(&mut self, start: NodeId, end: NodeId, reference: NodeId) {
        let (s, e, r) = (self.node(start), self.node(end), self.node(reference));
        let Some(parent) = r.parent_node() else {
            return;
        };
        let mut cur = Some(s.clone());
        while let Some(n) = cur {
            let last = &n == e;
            cur = n.next_sibling();
            let _ = parent.insert_before(&n, Some(r));
            if last {
                break;
            }
        }
    }

    fn listen(&mut self, node: NodeId, event: &str, handler: Handler) -> ListenerId {
        let id = self.next_listener;
        self.next_listener += 1;
        self.add_listener(id, node, event, handler);
        ListenerId(id)
    }

    fn unlisten(&mut self, id: ListenerId) {
        if let Some(l) = self.listeners.remove(&id.0) {
            let _ = l.node.target().remove_event_listener(&l.event, &l.closure);
            bury(l.closure);
        }
    }

    fn release(&mut self, nodes: &[NodeId]) {
        for n in nodes {
            if self.nodes[n.0 as usize].take().is_some() {
                self.raw_html.remove(&n.0);
                self.free.push(n.0);
            }
        }
    }

    fn set_title(&mut self, title: &str) {
        document().set_title(title);
    }

    fn element_by_id(&mut self, id: &str) -> Option<NodeId> {
        let found = self.doc.get_element_by_id(id)?;
        if let Some(&known) = self.by_id.get(id) {
            if self.live(known) == Some(&found) {
                return Some(known);
            }
            self.release(&[known]);
        }
        let node = self.adopt(found);
        self.by_id.insert(id.to_string(), node);
        Some(node)
    }

    fn rect(&mut self, node: NodeId) -> Option<crate::ui::Rect> {
        let r = self.live(node)?.get_bounding_client_rect();
        let n = |k: &str| dom::get_number(&r, k);
        Some(crate::ui::Rect {
            left: n("left"),
            top: n("top"),
            right: n("right"),
            bottom: n("bottom"),
        })
    }

    fn focus(&mut self, node: NodeId, on: bool) {
        if let Some(n) = self.live(node) {
            if on {
                n.focus();
            } else {
                n.blur();
            }
        }
    }

    fn scroll_into_view(&mut self, node: NodeId) {
        if let Some(n) = self.live(node) {
            dom::scroll_nearest(n);
        }
    }

    fn js_value(&mut self, node: NodeId) -> Option<wasm_bindgen::JsValue> {
        self.live(node).map(|n| n.clone().into())
    }

    fn head_tag(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        let doc = document();
        let Ok(Some(head)) = doc.query_selector("head") else {
            return;
        };
        let selector = format!(
            "head > {tag}{}",
            attrs
                .iter()
                .map(|(k, v)| format!("[{k}=\"{}\"]", v.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect::<String>()
        );
        if doc.query_selector(&selector).ok().flatten().is_some() {
            return;
        }
        let Ok(el) = doc.create_element(tag) else {
            return;
        };
        for (k, v) in attrs {
            let _ = el.set_attribute(k, v);
        }
        let _ = head.append_child(&el);
    }
}

fn nth_child(node: &Node, index: u8) -> Option<Node> {
    if index <= 2 {
        let mut child = node.first_child();
        for _ in 0..index {
            child = child?.next_sibling();
        }
        child
    } else {
        node.child_nodes().item(index as u32)
    }
}

/// Text downloaded before the client started for `head::boot(name, url)`, if the download worked
pub fn boot(name: &str) -> Option<String> {
    let window = dom::window()?;
    let all = dom::get_object(&window, "__yawnBoot")?;
    dom::get_string(&all, name)
}

struct Dom;

impl Real for Dom {
    type Node = Node;

    fn first_child(&self, node: &Node) -> Option<Node> {
        node.first_child()
    }

    fn next_sibling(&self, node: &Node) -> Option<Node> {
        node.next_sibling()
    }

    fn kind(&self, node: &Node) -> RealKind {
        match node.node_type() {
            dom::ELEMENT_NODE => RealKind::Element(node.tag_name().to_ascii_lowercase()),
            dom::TEXT_NODE => RealKind::Text(node.node_value().unwrap_or_default()),
            dom::COMMENT_NODE => RealKind::Comment,
            _ => RealKind::Other,
        }
    }

    fn split_text(&mut self, node: &Node, at: usize) -> Node {
        node.split_text(at as u32).expect("yawn: split_text")
    }

    fn insert_text(&mut self, parent: &Node, before: Option<&Node>, text: &str) -> Node {
        let node = document().create_text_node(text);
        let _ = parent.insert_before(&node, before);
        node
    }

    fn insert_comment(&mut self, parent: &Node, before: Option<&Node>) -> Node {
        let node = document().create_comment("");
        let _ = parent.insert_before(&node, before);
        node
    }
}

fn hydrate<C: Component>(host: &Node, root: C) -> bool {
    let app = crate::ui::mock::mount(root);
    let Some(table) = hydrate::adopt(&mut Dom, &app, host) else {
        return false;
    };
    // What the client computed wins over what the markup says, where they differ.
    for (i, slot) in table.iter().enumerate() {
        let Some(el) = slot.as_ref().filter(|n| n.node_type() == dom::ELEMENT_NODE) else {
            continue;
        };
        let id = NodeId(i as u32);
        for (k, v) in app.attrs(id) {
            if el.get_attribute(&k).as_deref() != Some(v.as_str()) {
                let _ = el.set_attribute(&k, &v);
            }
        }
        for (k, v) in app.props(id) {
            dom::set_string(el, &k, &v);
        }
    }
    // the development client patches literals in place: it needs the real nodes of what the build registered
    if has_dev_literals() {
        for (kind, original, current, id) in app.hydration_literals() {
            if let Some(node) = table.get(id.0 as usize).and_then(|n| n.as_ref()) {
                dom::dev_track_literal(&kind, &original, &current, node);
            }
        }
    }
    let mut backend = WebBackend::from_table(table, app.next_listener_id());
    for (id, node, event, handler) in app.hydration_listeners() {
        backend.add_listener(id.0, node, &event, handler);
    }
    crate::ui::rt::swap_backend(Box::new(backend));
    true
}

/// Starts the client: installs a panic hook that logs to the console and mounts `root` into `#app`,
/// replacing its content
pub fn start<C: Component + 'static>(root: C) {
    let mut root = Some(root);
    start_with(move || root.take().expect("yawn: the root is built once"));
}

/// Starts the client with a root that can be built again
pub fn start_with<C: Component + 'static>(make: impl FnMut() -> C + 'static) {
    std::panic::set_hook(Box::new(|info| {
        let text = info.to_string();
        dom::error(&text);
        dom::show_panic(&text);
    }));
    let host = document().get_element_by_id("app").expect("missing <div id=\"app\">");
    #[cfg(target_arch = "wasm32")]
    crate::ui::keep::embed();
    let make = std::rc::Rc::new(RefCell::new(make));
    let build = {
        let make = make.clone();
        move || (make.borrow_mut())()
    };
    let build_again = build.clone();
    REMOUNT.with(|r| {
        *r.borrow_mut() = Some(Box::new(move || {
            let Some(host) = document().get_element_by_id("app") else { return };
            host.set_text_content(None);
            let mut backend = WebBackend::new();
            let id = backend.adopt(host);
            crate::ui::mount(Box::new(backend), id, build_again());
        }))
    });
    // a client replaced by a hot reload finds the page as the old one left it, so there's no server markup to adopt
    let replaced = dom::window().is_some_and(|win| dom::get_number(&win, "__yawnReplaced") == 1.0);
    if host.has_child_nodes() && !replaced {
        if hydrate(&host, build()) {
            return;
        }
        dom::warn(&format!(
            "yawn: the page does not match what the client builds; rendering it again ({})",
            crate::ui::hydrate::mismatch_reason()
        ));
        // the attempt's listeners and timers go with it, or they would outlive their closures
        crate::ui::rt::dispose_document();
        crate::ui::keep::restart();
    }
    host.set_text_content(None);
    let mut backend = WebBackend::new();
    let id = backend.adopt(host);
    crate::ui::mount(Box::new(backend), id, build());
}

/// Declares the client entry point: `yawn::client!(counter::Counter);` mounts the component (built
/// with `Default`, so it takes no props) into `#app`
#[macro_export]
macro_rules! client {
    ($component:path) => {
        #[$crate::web::wasm_bindgen::prelude::wasm_bindgen(start, wasm_bindgen = $crate::web::wasm_bindgen)]
        pub fn __yawn_start() {
            $crate::web::start_with(<$component as ::core::default::Default>::default);
        }
        #[$crate::web::wasm_bindgen::prelude::wasm_bindgen(wasm_bindgen = $crate::web::wasm_bindgen)]
        pub fn __yawn_dispose() {
            $crate::web::dispose();
        }
        #[$crate::web::wasm_bindgen::prelude::wasm_bindgen(wasm_bindgen = $crate::web::wasm_bindgen)]
        pub fn __yawn_keep() {
            $crate::web::keep();
        }
    };
}

thread_local! {
    static VALUES: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
    static OVERRIDES: Option<wasm_bindgen::JsValue> = dom::window().and_then(|w| dom::get_object(&w, "__yawnLiterals"));
}

pub(crate) fn has_dev_literals() -> bool {
    OVERRIDES.with(|all| all.is_some())
}

pub(crate) fn dev_attr_literal(name: &str, value: &'static str) -> &'static str {
    if OVERRIDES.with(|all| all.is_none()) {
        return value;
    }
    dev_literal(&format!("attr:{name}"), value)
}

pub(crate) fn dev_literal(kind: &str, value: &'static str) -> &'static str {
    if OVERRIDES.with(|all| all.is_none()) {
        return value;
    }
    let key = format!("{kind}:{value}");
    let Some(current) =
        OVERRIDES.with(|all| all.as_ref().and_then(|all| dom::get_string(all, &key)))
    else {
        return value;
    };
    VALUES.with(|values| {
        *values
            .borrow_mut()
            .entry(current.clone())
            .or_insert_with(|| Box::leak(current.into_boxed_str()))
    })
}

thread_local! {
    static REMOUNT: RefCell<Option<Box<dyn FnMut()>>> = RefCell::new(None);
}

/// Dev: builds the application again in the page it is in, with the signals' values kept. Used
/// after a module with new bodies was loaded
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn remount() {
    let Some(mut again) = REMOUNT.with(|r| r.borrow_mut().take()) else { return };
    crate::ui::keep::carry(|| {
        crate::ui::rt::dispose_document();
        again();
    });
    REMOUNT.with(|r| *r.borrow_mut() = Some(again));
}

/// Dev: saves the signals' values for the next page load, without ending this client
pub fn keep() {
    crate::ui::keep::save();
}

pub fn dispose() {
    crate::ui::keep::save();
    crate::ui::rt::dispose_document();
}
