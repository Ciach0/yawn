#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use super::backend::NodeId;
use super::mock::{MockApp, Shape};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RealKind {
    /// Tag name in lower case
    Element(String),
    Text(String),
    Comment,
    Other,
}

pub(crate) trait Real {
    type Node: Clone;
    fn first_child(&self, node: &Self::Node) -> Option<Self::Node>;
    fn next_sibling(&self, node: &Self::Node) -> Option<Self::Node>;
    fn kind(&self, node: &Self::Node) -> RealKind;
    /// Splits a text node after `at` UTF-16 units; the rest becomes the next sibling and is
    /// returned
    fn split_text(&mut self, node: &Self::Node, at: usize) -> Self::Node;
    fn insert_text(
        &mut self,
        parent: &Self::Node,
        before: Option<&Self::Node>,
        text: &str,
    ) -> Self::Node;
    fn insert_comment(&mut self, parent: &Self::Node, before: Option<&Self::Node>) -> Self::Node;
}

pub(crate) fn adopt<R: Real>(
    real: &mut R,
    app: &MockApp,
    host: &R::Node,
) -> Option<Vec<Option<R::Node>>> {
    let mut table: Vec<Option<R::Node>> = vec![None; app.node_count()];
    table[app.host().0 as usize] = Some(host.clone());
    if !fill(real, app, app.host(), host, &mut table) {
        return None;
    }
    // Nodes the renderer released while building aren't its any more.
    for (i, slot) in table.iter_mut().enumerate() {
        if app.is_dead(NodeId(i as u32)) {
            *slot = None;
        }
    }
    Some(table)
}

thread_local! {
    static WHY: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub(crate) fn mismatch_reason() -> String {
    WHY.with(|w| w.borrow().clone())
}

fn differ(why: String) -> bool {
    WHY.with(|w| *w.borrow_mut() = why);
    false
}

fn fill<R: Real>(
    real: &mut R,
    app: &MockApp,
    mock: NodeId,
    parent: &R::Node,
    table: &mut Vec<Option<R::Node>>,
) -> bool {
    let mut cursor = real.first_child(parent);
    for child in app.children(mock) {
        match app.shape(child) {
            Shape::Comment => {
                table[child.0 as usize] = Some(real.insert_comment(parent, cursor.as_ref()));
            }
            Shape::Text(t) if t.is_empty() => {
                table[child.0 as usize] = Some(real.insert_text(parent, cursor.as_ref(), ""));
            }
            Shape::Text(t) => {
                let Some(current) = cursor.clone() else {
                    return differ(format!(
                        "the client has the text {t:?}, the page has nothing more here"
                    ));
                };
                let RealKind::Text(data) = real.kind(&current) else {
                    return differ(format!(
                        "the client has the text {t:?}, the page has an element or comment"
                    ));
                };
                if !data.starts_with(&t) {
                    return differ(format!("the client has the text {t:?}, the page has {data:?}"));
                }
                cursor = if data.len() > t.len() {
                    Some(real.split_text(&current, t.encode_utf16().count()))
                } else {
                    real.next_sibling(&current)
                };
                // `current` now holds exactly `t` (the split keeps the front in place).
                table[child.0 as usize] = Some(current);
            }
            Shape::Element(tag) => {
                let Some(current) = cursor.clone() else {
                    return differ(format!(
                        "the client has <{tag}>, the page has nothing more here"
                    ));
                };
                match real.kind(&current) {
                    RealKind::Element(t) if t == tag.to_ascii_lowercase() => {}
                    other => {
                        return differ(format!("the client has <{tag}>, the page has {other:?}"));
                    }
                }
                table[child.0 as usize] = Some(current.clone());
                let raw = app.props(child).iter().any(|(name, _)| name == "innerHTML");
                if !raw && !fill(real, app, child, &current, table) {
                    return false;
                }
                cursor = real.next_sibling(&current);
            }
        }
    }
    // markup the client doesn't build
    cursor.is_none() || differ("the page has markup the client does not build".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Component, View, mock, rt, signal};

    #[derive(Default)]
    struct Copy {
        nodes: Vec<(Kind, Vec<usize>)>,
    }

    enum Kind {
        El(String),
        Text(String),
        Comment,
    }

    const VOID: [&str; 4] = ["input", "br", "img", "hr"];

    impl Copy {
        fn parse(html: &str) -> Copy {
            let mut c = Copy { nodes: vec![(Kind::El("div".into()), Vec::new())] };
            let mut stack = vec![0usize];
            let mut rest = html;
            while !rest.is_empty() {
                let top = *stack.last().unwrap();
                if let Some(tail) = rest.strip_prefix("</") {
                    let end = tail.find('>').unwrap();
                    stack.pop();
                    rest = &tail[end + 1..];
                } else if let Some(tail) = rest.strip_prefix('<') {
                    let end = tail.find('>').unwrap();
                    let tag = tail[..end].split_whitespace().next().unwrap().to_string();
                    let id = c.add(top, Kind::El(tag.clone()));
                    if !VOID.contains(&tag.as_str()) {
                        stack.push(id);
                    }
                    rest = &tail[end + 1..];
                } else {
                    let end = rest.find('<').unwrap_or(rest.len());
                    let text =
                        rest[..end].replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
                    c.add(top, Kind::Text(text));
                    rest = &rest[end..];
                }
            }
            c
        }

        fn add(&mut self, parent: usize, kind: Kind) -> usize {
            self.nodes.push((kind, Vec::new()));
            let id = self.nodes.len() - 1;
            self.nodes[parent].1.push(id);
            id
        }

        fn insert(&mut self, parent: usize, before: Option<&usize>, kind: Kind) -> usize {
            self.nodes.push((kind, Vec::new()));
            let id = self.nodes.len() - 1;
            let kids = &mut self.nodes[parent].1;
            match before.and_then(|b| kids.iter().position(|k| k == b)) {
                Some(i) => kids.insert(i, id),
                None => kids.push(id),
            }
            id
        }

        fn parent_of(&self, node: usize) -> usize {
            self.nodes.iter().position(|(_, kids)| kids.contains(&node)).unwrap()
        }

        fn write(&self, id: usize, out: &mut String) {
            match &self.nodes[id].0 {
                Kind::Text(t) => {
                    out.push_str(&t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"))
                }
                Kind::Comment => out.push_str("<!---->"),
                Kind::El(tag) => {
                    out.push_str(&format!("<{tag}>"));
                    if VOID.contains(&tag.as_str()) {
                        return;
                    }
                    for k in &self.nodes[id].1 {
                        self.write(*k, out);
                    }
                    out.push_str(&format!("</{tag}>"));
                }
            }
        }

        fn html(&self) -> String {
            let mut out = String::new();
            for k in &self.nodes[0].1 {
                self.write(*k, &mut out);
            }
            out
        }

        fn texts(&self) -> usize {
            self.nodes.iter().filter(|(k, _)| matches!(k, Kind::Text(_))).count()
        }
    }

    impl Real for Copy {
        type Node = usize;
        fn first_child(&self, n: &usize) -> Option<usize> {
            self.nodes[*n].1.first().copied()
        }
        fn next_sibling(&self, n: &usize) -> Option<usize> {
            let kids = &self.nodes[self.parent_of(*n)].1;
            kids.get(kids.iter().position(|k| k == n)? + 1).copied()
        }
        fn kind(&self, n: &usize) -> RealKind {
            match &self.nodes[*n].0 {
                Kind::El(t) => RealKind::Element(t.clone()),
                Kind::Text(t) => RealKind::Text(t.clone()),
                Kind::Comment => RealKind::Comment,
            }
        }
        fn split_text(&mut self, n: &usize, at: usize) -> usize {
            let Kind::Text(t) = &self.nodes[*n].0 else { panic!("not text") };
            let units: Vec<u16> = t.encode_utf16().collect();
            let (front, back) = (
                String::from_utf16(&units[..at]).unwrap(),
                String::from_utf16(&units[at..]).unwrap(),
            );
            self.nodes[*n].0 = Kind::Text(front);
            let parent = self.parent_of(*n);
            let next = self.next_sibling(n);
            self.insert(parent, next.as_ref(), Kind::Text(back))
        }
        fn insert_text(&mut self, parent: &usize, before: Option<&usize>, text: &str) -> usize {
            self.insert(*parent, before, Kind::Text(text.to_string()))
        }
        fn insert_comment(&mut self, parent: &usize, before: Option<&usize>) -> usize {
            self.insert(*parent, before, Kind::Comment)
        }
    }

    struct Page;
    impl Component for Page {
        fn render(self) -> View {
            let root = rt::element("section");
            let h = rt::element("h1");
            // adjacent texts, one of them empty at first: the parser would make one node of them
            rt::add(h, rt::text("Hello, "));
            let name = signal("world".to_string());
            rt::add(h, rt::text_fmt(move || name.get()));
            rt::add(h, rt::text(" & <you>"));
            rt::add(root, h);
            let on = signal(true);
            rt::add(
                root,
                rt::show(
                    move || on.get(),
                    || {
                        let s = rt::element("span");
                        rt::add(s, rt::text("yes"));
                        s.into()
                    },
                    View::empty,
                ),
            );
            rt::add(
                root,
                rt::each(
                    || vec![1, 2, 3],
                    |n| *n,
                    |n| {
                        let li = rt::element("li");
                        rt::add(li, rt::text(&format!("row {n}")));
                        li.into()
                    },
                ),
            );
            rt::add(root, rt::element("input"));
            rt::add(root, rt::text(""));
            root.into()
        }
    }

    fn server_copy(app: &mock::MockApp) -> Copy {
        Copy::parse(&app.html())
    }

    #[test]
    fn adopts_server_markup_and_restores_what_html_cannot_carry() {
        let app = mock::mount(Page);
        let mut real = server_copy(&app);
        assert_eq!(real.texts(), 5, "the parser makes one text of `h1`'s three: {}", real.html());
        let table = adopt(&mut real, &app, &0).expect("same markup");
        // anchors and empty text are back: the real tree now equals the client's, comments included
        assert_eq!(real.html(), app.html_with_anchors());
        // every live id points at a node of the right kind
        for (i, slot) in table.iter().enumerate() {
            let id = NodeId(i as u32);
            match (app.is_dead(id), slot) {
                (true, slot) => assert!(slot.is_none()),
                (false, Some(n)) => match app.shape(id) {
                    Shape::Element(t) => assert_eq!(real.kind(n), RealKind::Element(t)),
                    Shape::Text(t) => assert_eq!(real.kind(n), RealKind::Text(t)),
                    Shape::Comment => assert_eq!(real.kind(n), RealKind::Comment),
                },
                (false, None) => panic!("live node {i} was not adopted"),
            }
        }
    }

    #[test]
    fn markup_that_differs_is_refused() {
        let app = mock::mount(Page);
        let html = app.html();
        for broken in [
            html.replace("<h1>", "<h2>").replace("</h1>", "</h2>"),
            html.replace("yes", "no"),
            format!("{html}<p>extra</p>"),
            html.replace("<li>row 3</li>", ""),
        ] {
            let mut real = Copy::parse(&broken);
            assert!(adopt(&mut real, &app, &0).is_none(), "{broken}");
        }
    }
}
