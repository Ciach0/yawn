use super::Component;
use super::backend::{Backend, Event, Handler, ListenerId, NodeId, Rect};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

enum Kind {
    Element { tag: String, attrs: Vec<(String, String)>, props: BTreeMap<String, String> },
    Text(String),
    Comment,
}

struct Node {
    kind: Kind,
    dead: bool,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

#[derive(Default)]
pub struct MockDom {
    nodes: Vec<Node>,
    listeners: BTreeMap<u32, (NodeId, String, Handler)>,
    next_listener: u32,
    title: String,
    head: Vec<(String, Vec<(String, String)>)>,
    /// Backend calls that changed the document, to assert that nothing else was touched
    pub mutations: usize,
    literals: Vec<(String, String, String, NodeId)>,
    rects: BTreeMap<u32, Rect>,
    focused: Option<NodeId>,
    scrolled: Vec<NodeId>,
}

impl MockDom {
    fn raw(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    fn node(&self, id: NodeId) -> &Node {
        let n = self.raw(id);
        assert!(!n.dead, "use of released node {}", id.0);
        n
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        let n = &mut self.nodes[id.0 as usize];
        assert!(!n.dead, "use of released node {}", id.0);
        n
    }

    fn add(&mut self, kind: Kind) -> NodeId {
        self.nodes.push(Node { kind, dead: false, parent: None, children: Vec::new() });
        NodeId(self.nodes.len() as u32 - 1)
    }

    fn detach(&mut self, id: NodeId) {
        if let Some(p) = self.raw(id).parent {
            self.nodes[p.0 as usize].children.retain(|c| *c != id);
            self.nodes[id.0 as usize].parent = None;
        }
    }

    fn attach(&mut self, parent: NodeId, index: Option<usize>, id: NodeId) {
        self.detach(id);
        let children = &mut self.node_mut(parent).children;
        match index {
            Some(i) => children.insert(i, id),
            None => children.push(id),
        }
        self.node_mut(id).parent = Some(parent);
    }

    fn position(&self, id: NodeId) -> (NodeId, usize) {
        let p = self.raw(id).parent.expect("node has no parent");
        (p, self.raw(p).children.iter().position(|c| *c == id).expect("child of its parent"))
    }

    fn range(&self, start: NodeId, end: NodeId) -> Vec<NodeId> {
        let (p, a) = self.position(start);
        let (p2, b) = self.position(end);
        assert_eq!(p, p2, "range ends have different parents");
        self.raw(p).children[a..=b].to_vec()
    }

    fn write_html(&self, id: NodeId, comments: bool, out: &mut String) {
        let n = self.raw(id);
        match &n.kind {
            Kind::Text(t) => {
                out.push_str(&t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"))
            }
            Kind::Comment => {
                if comments {
                    out.push_str("<!---->");
                }
            }
            Kind::Element { tag, attrs, props } => {
                out.push('<');
                out.push_str(tag);
                for (k, v) in attrs {
                    if v.is_empty() {
                        out.push_str(&format!(" {k}"));
                    } else {
                        out.push_str(&format!(
                            " {k}=\"{}\"",
                            v.replace('&', "&amp;").replace('"', "&quot;")
                        ));
                    }
                }
                out.push('>');
                if matches!(
                    tag.as_str(),
                    "input"
                        | "br"
                        | "hr"
                        | "img"
                        | "meta"
                        | "link"
                        | "area"
                        | "base"
                        | "col"
                        | "embed"
                        | "source"
                        | "track"
                        | "wbr"
                ) {
                    return;
                }
                if let Some(inner) = props.get("innerHTML") {
                    out.push_str(inner);
                } else {
                    for c in &n.children {
                        self.write_html(*c, comments, out);
                    }
                }
                out.push_str(&format!("</{tag}>"));
            }
        }
    }
}

struct MockBackend(Rc<RefCell<MockDom>>);

impl Backend for MockBackend {
    fn create_element(&mut self, tag: &str) -> NodeId {
        self.0.borrow_mut().add(Kind::Element {
            tag: tag.to_string(),
            attrs: Vec::new(),
            props: BTreeMap::new(),
        })
    }
    fn create_text(&mut self, text: &str) -> NodeId {
        self.0.borrow_mut().add(Kind::Text(text.to_string()))
    }
    fn track_literal(&mut self, kind: &str, original: &str, current: &str, node: NodeId) {
        self.0.borrow_mut().literals.push((
            kind.to_string(),
            original.to_string(),
            current.to_string(),
            node,
        ));
    }
    fn create_comment(&mut self) -> NodeId {
        self.0.borrow_mut().add(Kind::Comment)
    }
    fn set_text(&mut self, node: NodeId, text: &str) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        match &mut d.node_mut(node).kind {
            Kind::Text(t) => *t = text.to_string(),
            _ => panic!("set_text on a non-text node"),
        }
    }
    fn set_attr(&mut self, node: NodeId, name: &str, value: &str) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        if let Kind::Element { attrs, .. } = &mut d.node_mut(node).kind {
            match attrs.iter_mut().find(|(k, _)| k == name) {
                Some(a) => a.1 = value.to_string(),
                None => attrs.push((name.to_string(), value.to_string())),
            }
        }
    }
    fn remove_attr(&mut self, node: NodeId, name: &str) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        if let Kind::Element { attrs, .. } = &mut d.node_mut(node).kind {
            attrs.retain(|(k, _)| k != name);
        }
    }
    fn set_prop(&mut self, node: NodeId, name: &str, value: &str) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        if let Kind::Element { props, .. } = &mut d.node_mut(node).kind {
            props.insert(name.to_string(), value.to_string());
        }
    }
    fn set_class(&mut self, node: NodeId, class: &str, on: bool) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        if let Kind::Element { attrs, .. } = &mut d.node_mut(node).kind {
            let current = attrs
                .iter()
                .find(|(k, _)| k == "class")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let mut tokens: Vec<&str> = current.split_whitespace().collect();
            let has = tokens.contains(&class);
            if on && !has {
                tokens.push(class);
            } else if !on {
                tokens.retain(|t| *t != class);
            }
            let joined = tokens.join(" ");
            attrs.retain(|(k, _)| k != "class");
            if !joined.is_empty() {
                attrs.push(("class".to_string(), joined));
            }
        }
    }
    fn append(&mut self, parent: NodeId, child: NodeId) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        d.node(parent);
        d.node(child);
        d.attach(parent, None, child);
    }
    fn insert_before(&mut self, reference: NodeId, node: NodeId) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        d.node(node);
        d.node(reference);
        d.detach(node);
        let (p, i) = d.position(reference);
        d.attach(p, Some(i), node);
    }
    fn remove(&mut self, node: NodeId) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        d.node(node);
        d.detach(node);
    }
    fn remove_between(&mut self, start: NodeId, end: NodeId) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        d.node(start);
        d.node(end);
        let all = d.range(start, end);
        for n in &all[1..all.len() - 1] {
            d.detach(*n);
        }
    }
    fn remove_range(&mut self, start: NodeId, end: NodeId) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        d.node(start);
        d.node(end);
        for n in d.range(start, end) {
            d.detach(n);
        }
    }
    fn move_range(&mut self, start: NodeId, end: NodeId, reference: NodeId) {
        let mut d = self.0.borrow_mut();
        d.mutations += 1;
        d.node(start);
        d.node(end);
        d.node(reference);
        let nodes = d.range(start, end);
        for n in nodes {
            d.detach(n);
            let (p, i) = d.position(reference);
            d.attach(p, Some(i), n);
        }
    }
    fn listen(&mut self, node: NodeId, event: &str, handler: Handler) -> ListenerId {
        let mut d = self.0.borrow_mut();
        let id = d.next_listener;
        d.next_listener += 1;
        d.listeners.insert(id, (node, event.to_string(), handler));
        ListenerId(id)
    }
    fn unlisten(&mut self, id: ListenerId) {
        self.0.borrow_mut().listeners.remove(&id.0);
    }
    fn release(&mut self, nodes: &[NodeId]) {
        let mut d = self.0.borrow_mut();
        for n in nodes {
            assert!(!d.nodes[n.0 as usize].dead, "node {} released twice", n.0);
            d.nodes[n.0 as usize].dead = true;
        }
    }
    fn set_title(&mut self, title: &str) {
        self.0.borrow_mut().title = title.to_string();
    }
    fn element_by_id(&mut self, id: &str) -> Option<NodeId> {
        let d = self.0.borrow();
        d.nodes.iter().enumerate().find_map(|(i, n)| match &n.kind {
            Kind::Element { attrs, .. }
                if !n.dead && attrs.iter().any(|(k, v)| k == "id" && v == id) =>
            {
                Some(NodeId(i as u32))
            }
            _ => None,
        })
    }
    fn rect(&mut self, node: NodeId) -> Option<Rect> {
        self.0.borrow().rects.get(&node.0).copied()
    }
    fn focus(&mut self, node: NodeId, on: bool) {
        let mut d = self.0.borrow_mut();
        if on {
            d.focused = Some(node);
        } else if d.focused == Some(node) {
            d.focused = None;
        }
    }
    fn scroll_into_view(&mut self, node: NodeId) {
        self.0.borrow_mut().scrolled.push(node);
    }
    fn head_tag(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        let attrs: Vec<(String, String)> =
            attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut dom = self.0.borrow_mut();
        if !dom.head.iter().any(|(t, a)| t == tag && *a == attrs) {
            dom.head.push((tag.to_string(), attrs));
        }
    }
}

/// A mounted app and its in-memory document
pub struct MockApp {
    dom: Rc<RefCell<MockDom>>,
    host: NodeId,
}

/// Mounts `root` into a fresh `<div id="app">`
pub fn mount<C: Component>(root: C) -> MockApp {
    let dom = Rc::new(RefCell::new(MockDom::default()));
    let host = {
        let mut b = MockBackend(dom.clone());
        let host = b.create_element("div");
        b.set_attr(host, "id", "app");
        host
    };
    super::mount(Box::new(MockBackend(dom.clone())), host, root);
    dom.borrow_mut().mutations = 0;
    MockApp { dom, host }
}

impl MockApp {
    /// HTML of the app without internal anchor comments
    pub fn html(&self) -> String {
        let d = self.dom.borrow();
        let mut out = String::new();
        for c in &d.raw(self.host).children {
            d.write_html(*c, false, &mut out);
        }
        out
    }

    pub fn title(&self) -> String {
        self.dom.borrow().title.clone()
    }

    pub(crate) fn head_html(&self) -> String {
        let d = self.dom.borrow();
        let mut out = String::new();
        for (tag, attrs) in &d.head {
            out.push('<');
            out.push_str(tag);
            for (k, v) in attrs {
                out.push_str(&format!(
                    " {k}=\"{}\"",
                    v.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
                ));
            }
            out.push_str(">\n");
        }
        out
    }

    /// Elements matching a selector built from `tag`, `.class` and `#id` compounds separated by
    /// spaces (descendants), in document order: `ul.log li`
    pub fn find_all(&self, selector: &str) -> Vec<NodeId> {
        fn matches(d: &MockDom, n: NodeId, compound: &str) -> bool {
            let Kind::Element { tag, attrs, .. } = &d.raw(n).kind else {
                return false;
            };
            let get =
                |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()).unwrap_or("");
            let mut rest = compound;
            let name_len = rest.find(['.', '#']).unwrap_or(rest.len());
            if name_len > 0 && &rest[..name_len] != tag {
                return false;
            }
            rest = &rest[name_len..];
            while let Some(c) = rest.chars().next() {
                let end = rest[1..].find(['.', '#']).map(|i| i + 1).unwrap_or(rest.len());
                let want = &rest[1..end];
                let ok = if c == '.' {
                    get("class").split_whitespace().any(|x| x == want)
                } else {
                    get("id") == want
                };
                if !ok {
                    return false;
                }
                rest = &rest[end..];
            }
            true
        }
        let compounds: Vec<&str> = selector.split_whitespace().collect();
        let Some((last, ancestors)) = compounds.split_last() else {
            return Vec::new();
        };
        let d = self.dom.borrow();
        let mut out = Vec::new();
        fn walk(d: &MockDom, n: NodeId, f: &mut dyn FnMut(NodeId)) {
            for c in &d.raw(n).children {
                f(*c);
                walk(d, *c, f);
            }
        }
        walk(&d, self.host, &mut |n| {
            if !matches(&d, n, last) {
                return;
            }
            let mut need = ancestors.iter().rev();
            let mut want = need.next();
            let mut at = d.raw(n).parent;
            while let (Some(w), Some(p)) = (want, at) {
                if matches(&d, p, w) {
                    want = need.next();
                }
                at = d.raw(p).parent;
            }
            if want.is_none() {
                out.push(n);
            }
        });
        out
    }

    pub fn find(&self, selector: &str) -> NodeId {
        *self
            .find_all(selector)
            .first()
            .unwrap_or_else(|| panic!("no element matches `{selector}`; html: {}", self.html()))
    }

    pub fn text(&self, node: NodeId) -> String {
        fn collect(d: &MockDom, n: NodeId, out: &mut String) {
            match &d.raw(n).kind {
                Kind::Text(t) => out.push_str(t),
                _ => d.raw(n).children.iter().for_each(|c| collect(d, *c, out)),
            }
        }
        let d = self.dom.borrow();
        let mut out = String::new();
        collect(&d, node, &mut out);
        out
    }

    pub fn attr(&self, node: NodeId, name: &str) -> Option<String> {
        match &self.dom.borrow().raw(node).kind {
            Kind::Element { attrs, .. } => {
                attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
            }
            _ => None,
        }
    }

    pub fn prop(&self, node: NodeId, name: &str) -> Option<String> {
        match &self.dom.borrow().raw(node).kind {
            Kind::Element { props, .. } => props.get(name).cloned(),
            _ => None,
        }
    }

    pub fn dispatch(&self, node: NodeId, event: &str, ev: Event) {
        let handlers: Vec<Handler> = self
            .dom
            .borrow()
            .listeners
            .values()
            .filter(|(n, e, _)| *n == node && e == event)
            .map(|(_, _, h)| h.clone())
            .collect();
        for h in handlers {
            (h.borrow_mut())(ev.clone());
        }
    }

    pub fn click(&self, node: NodeId) {
        self.dispatch(node, "click", Event::default());
    }

    pub fn input(&self, node: NodeId, value: &str) {
        self.dispatch(node, "input", Event { value: value.to_string(), ..Event::default() });
    }

    /// Nodes the renderer created and hasn't released (anchors and everything on the page)
    pub fn live_nodes(&self) -> usize {
        self.dom.borrow().nodes.iter().filter(|n| !n.dead).count()
    }

    pub fn listeners(&self) -> usize {
        self.dom.borrow().listeners.len()
    }

    /// Makes `rect()` of the element answer with `rect`, as a browser's layout would
    pub fn set_rect(&self, node: NodeId, rect: Rect) {
        self.dom.borrow_mut().rects.insert(node.0, rect);
    }

    /// The element that holds keyboard focus
    pub fn focused(&self) -> Option<NodeId> {
        self.dom.borrow().focused
    }

    /// The elements `scroll_into_view` was called on, in order
    pub fn scrolled(&self) -> Vec<NodeId> {
        self.dom.borrow().scrolled.clone()
    }

    /// Number of document mutations since mount (or since the last [`MockApp::reset_mutations`])
    pub fn mutations(&self) -> usize {
        self.dom.borrow().mutations
    }

    pub fn reset_mutations(&self) {
        self.dom.borrow_mut().mutations = 0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Element(String),
    Text(String),
    Comment,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl MockApp {
    pub(crate) fn host(&self) -> NodeId {
        self.host
    }

    pub(crate) fn node_count(&self) -> usize {
        self.dom.borrow().nodes.len()
    }

    pub(crate) fn is_dead(&self, id: NodeId) -> bool {
        self.dom.borrow().raw(id).dead
    }

    pub(crate) fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.dom.borrow().raw(id).children.clone()
    }

    pub(crate) fn shape(&self, id: NodeId) -> Shape {
        match &self.dom.borrow().raw(id).kind {
            Kind::Element { tag, .. } => Shape::Element(tag.clone()),
            Kind::Text(t) => Shape::Text(t.clone()),
            Kind::Comment => Shape::Comment,
        }
    }

    pub(crate) fn attrs(&self, id: NodeId) -> Vec<(String, String)> {
        match &self.dom.borrow().raw(id).kind {
            Kind::Element { attrs, .. } => attrs.clone(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn props(&self, id: NodeId) -> Vec<(String, String)> {
        match &self.dom.borrow().raw(id).kind {
            Kind::Element { props, .. } => {
                props.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn hydration_listeners(&self) -> Vec<(ListenerId, NodeId, String, Handler)> {
        self.dom
            .borrow()
            .listeners
            .iter()
            .map(|(id, (n, e, h))| (ListenerId(*id), *n, e.clone(), h.clone()))
            .collect()
    }

    pub(crate) fn hydration_literals(&self) -> Vec<(String, String, String, NodeId)> {
        self.dom.borrow().literals.clone()
    }

    pub(crate) fn next_listener_id(&self) -> u32 {
        self.dom.borrow().next_listener
    }

    #[cfg(test)]
    pub(crate) fn html_with_anchors(&self) -> String {
        let d = self.dom.borrow();
        let mut out = String::new();
        for c in &d.raw(self.host).children {
            d.write_html(*c, true, &mut out);
        }
        out
    }
}
