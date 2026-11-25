use super::backend::{Backend, NodeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Elements that have no end tag
pub const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

enum Op {
    Open(String, Vec<(String, String)>),
    Text(String),
    Close,
}

struct Parsed {
    ops: Vec<Op>,
    holes: Vec<usize>,
}

thread_local! {
    /// Parsed templates by the address of their (static) markup
    static PARSED: RefCell<HashMap<usize, Rc<Parsed>>> = RefCell::new(HashMap::new());
}

pub(crate) fn build<B: Backend + ?Sized>(backend: &mut B, html: &'static str) -> NodeId {
    let mut out = [NodeId(0)];
    build_holes(backend, html, &[&[]], &mut out);
    out[0]
}

pub(crate) fn build_holes<B: Backend + ?Sized>(
    backend: &mut B,
    html: &'static str,
    paths: &[&'static [u8]],
    out: &mut [NodeId],
) {
    let parsed = PARSED.with(|p| {
        p.borrow_mut()
            .entry(html.as_ptr() as usize)
            .or_insert_with(|| Rc::new(parse(html, paths)))
            .clone()
    });
    let mut created: Vec<NodeId> = Vec::with_capacity(parsed.ops.len() / 2 + 1);
    let mut stack: Vec<NodeId> = Vec::new();
    let mut root = None;
    for op in parsed.ops.iter() {
        match op {
            Op::Open(tag, attrs) => {
                let id = backend.create_element(tag);
                created.push(id);
                for (name, value) in attrs {
                    backend.set_attr(id, name, value);
                }
                match stack.last() {
                    Some(&parent) => backend.append(parent, id),
                    None => root = root.or(Some(id)),
                }
                if !VOID.contains(&tag.as_str()) {
                    stack.push(id);
                }
            }
            Op::Text(text) => {
                let id = backend.create_text(text);
                created.push(id);
                if let Some(&parent) = stack.last() {
                    backend.append(parent, id);
                }
            }
            Op::Close => {
                stack.pop();
            }
        }
    }
    root.expect("yawn: a template needs a root element");
    for (slot, &at) in out.iter_mut().zip(&parsed.holes) {
        *slot = created[at];
    }
    // The renderer only knows the nodes it was handed. The rest are part of the tree for good
    // (a browser backend doesn't even give them ids), so they're released right away.
    let rest: Vec<NodeId> = created
        .iter()
        .enumerate()
        .filter(|(i, _)| !parsed.holes.contains(i))
        .map(|(_, id)| *id)
        .collect();
    if !rest.is_empty() {
        backend.release(&rest);
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

fn parse(html: &str, paths: &[&[u8]]) -> Parsed {
    let mut ops = Vec::new();
    let mut holes = vec![usize::MAX; paths.len()];
    // where the next created node sits: the path of its parent and how many children that has so far
    let mut here: Vec<u8> = Vec::new();
    let mut counts: Vec<u8> = vec![0];
    let mut created = 0usize;
    let mut place = |here: &mut Vec<u8>, counts: &mut Vec<u8>, created: &mut usize, open: bool| {
        let index = *counts.last().unwrap();
        *counts.last_mut().unwrap() += 1;
        here.push(index);
        for (k, path) in paths.iter().enumerate() {
            // `here` starts with the root's own index; a path starts below the root
            if *path == &here[1..] {
                holes[k] = *created;
            }
        }
        *created += 1;
        if open {
            counts.push(0);
        } else {
            here.pop();
        }
    };
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("</") {
            let end = tail.find('>').expect("yawn: unterminated end tag in a template");
            ops.push(Op::Close);
            counts.pop();
            here.pop();
            rest = &tail[end + 1..];
        } else if let Some(tail) = rest.strip_prefix('<') {
            let (tag_end, name_end) = (
                tag_end(tail),
                tail.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(tail.len()),
            );
            let name = tail[..name_end].to_string();
            let mut attrs = Vec::new();
            let mut a = tail[name_end..tag_end].trim_start();
            while !a.is_empty() {
                let eq =
                    a.find("=\"").expect("yawn: attribute without a quoted value in a template");
                let value_end = a[eq + 2..]
                    .find('"')
                    .expect("yawn: unterminated attribute value in a template")
                    + eq
                    + 2;
                attrs.push((a[..eq].to_string(), unescape(&a[eq + 2..value_end])));
                a = a[value_end + 1..].trim_start();
            }
            place(&mut here, &mut counts, &mut created, !VOID.contains(&name.as_str()));
            ops.push(Op::Open(name, attrs));
            rest = &tail[tag_end + 1..];
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            place(&mut here, &mut counts, &mut created, false);
            ops.push(Op::Text(unescape(&rest[..end])));
            rest = &rest[end..];
        }
    }
    assert!(holes.iter().all(|&h| h != usize::MAX), "yawn: a path of a template leads nowhere");
    Parsed { ops, holes }
}

fn tag_end(tail: &str) -> usize {
    let mut quoted = false;
    for (i, c) in tail.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '>' if !quoted => return i,
            _ => {}
        }
    }
    panic!("yawn: unterminated start tag in a template")
}

#[cfg(test)]
mod tests {
    use crate::ui::{Component, View, mock, rt};

    struct T(&'static str);
    impl Component for T {
        fn render(self) -> View {
            rt::template(self.0).into()
        }
    }

    #[test]
    fn builds_the_same_tree_as_the_markup_describes() {
        let app = mock::mount(T(
            "<div class=\"a b\" data-x=\"1 &amp; 2\"><p>hi &lt;you&gt;</p><br><input disabled=\"\"><span></span></div>",
        ));
        // the server writes an empty attribute without `=""`, which means the same
        assert_eq!(
            app.html(),
            "<div class=\"a b\" data-x=\"1 &amp; 2\"><p>hi &lt;you&gt;</p><br><input disabled><span></span></div>"
        );
    }

    struct Holes;
    impl Component for Holes {
        fn render(self) -> View {
            const PATHS: &[&[u8]] = &[&[], &[0], &[1, 0]];
            let mut out = [crate::ui::NodeId(0); 3];
            rt::with_backend(|b| {
                b.create_template_holes("<div><span></span><b><i></i></b></div>", PATHS, &mut out)
            });
            for (id, text) in out.iter().zip(["root", "span", "i"]) {
                rt::add(*id, rt::text(text));
            }
            out[0].into()
        }
    }

    #[test]
    fn holes_are_the_nodes_the_paths_lead_to() {
        let app = mock::mount(Holes);
        assert_eq!(app.html(), "<div><span>span</span><b><i>i</i></b>root</div>");
    }

    #[test]
    fn the_same_template_can_be_built_twice() {
        const HTML: &str = "<ul><li>1</li><li>2</li></ul>";
        let a = mock::mount(T(HTML)).html();
        let b = mock::mount(T(HTML)).html();
        assert_eq!(a, "<ul><li>1</li><li>2</li></ul>");
        assert_eq!(a, b);
    }
}
