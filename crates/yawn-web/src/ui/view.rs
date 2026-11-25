use super::backend::NodeId;
use super::signals::{Memo, Sig};
use std::cell::RefCell;
use std::fmt::Display;
use std::rc::Rc;

/// The top-level nodes a piece of view produced
pub enum View {
    One(NodeId),
    Many(Vec<NodeId>),
}

impl View {
    /// Nothing: a view with no nodes
    pub fn empty() -> View {
        View::Many(Vec::new())
    }

    /// The top-level nodes, in order
    pub fn into_nodes(self) -> Vec<NodeId> {
        match self {
            View::One(n) => vec![n],
            View::Many(v) => v,
        }
    }
}

impl From<NodeId> for View {
    fn from(n: NodeId) -> View {
        View::One(n)
    }
}

/// Collects top-level nodes while a view is built
#[derive(Default)]
pub struct Frag {
    nodes: Vec<NodeId>,
}

impl Frag {
    pub fn new() -> Frag {
        Frag::default()
    }

    pub fn finish(mut self) -> View {
        if self.nodes.len() == 1 {
            View::One(self.nodes.pop().unwrap())
        } else {
            View::Many(self.nodes)
        }
    }
}

/// Somewhere built nodes can be put: an element, or a [`Frag`]
pub trait Parent {
    fn add(self, view: View);
}

impl Parent for NodeId {
    fn add(self, view: View) {
        super::rt::with_backend(|b| match view {
            View::One(n) => b.append(self, n),
            View::Many(v) => v.into_iter().for_each(|n| b.append(self, n)),
        });
    }
}

impl Parent for &mut Frag {
    fn add(self, view: View) {
        match view {
            View::One(n) => self.nodes.push(n),
            View::Many(v) => self.nodes.extend(v),
        }
    }
}

type Builder = Box<dyn FnMut() -> View>;

/// The content a component receives between its tags, or in a named slot (`slot="title"`)
#[derive(Clone)]
pub struct Children {
    build: Rc<RefCell<Builder>>,
}

impl Children {
    /// Content built when the component that receives it places it (`<slot />`)
    pub fn new(build: impl FnMut() -> View + 'static) -> Children {
        Children { build: Rc::new(RefCell::new(Box::new(build))) }
    }

    /// Builds the content now
    pub fn view(&self) -> View {
        (self.build.borrow_mut())()
    }
}

impl Default for Children {
    fn default() -> Self {
        Children::new(View::empty)
    }
}

/// What a `{expression}` in a view produced
pub enum Child {
    Text(String),
    View(View),
}

/// Types that can appear as `{value}` in a view
pub trait IntoChild {
    fn into_child(self) -> Child;
}

macro_rules! display_child {
    ($($t:ty),*) => {$(
        impl IntoChild for $t { fn into_child(self) -> Child { Child::Text(self.to_string()) } }
        impl AttrValue for $t { fn attr_value(self) -> Option<String> { Some(self.to_string()) } }
    )*};
}
display_child!(
    String, &str, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

impl IntoChild for bool {
    fn into_child(self) -> Child {
        Child::Text(self.to_string())
    }
}
impl IntoChild for View {
    fn into_child(self) -> Child {
        Child::View(self)
    }
}
impl IntoChild for Children {
    fn into_child(self) -> Child {
        Child::View(self.view())
    }
}
impl<C: IntoChild> IntoChild for Option<C> {
    fn into_child(self) -> Child {
        match self {
            Some(c) => c.into_child(),
            None => Child::Text(String::new()),
        }
    }
}
impl<T: Display + Clone + 'static> IntoChild for Sig<T> {
    fn into_child(self) -> Child {
        Child::Text(self.get().to_string())
    }
}
impl<T: Display + Clone + 'static> IntoChild for Memo<T> {
    fn into_child(self) -> Child {
        Child::Text(self.get().to_string())
    }
}

/// Types usable as an attribute value
pub trait AttrValue {
    fn attr_value(self) -> Option<String>;
}

impl AttrValue for bool {
    /// `true` sets a valueless attribute, `false` removes it
    fn attr_value(self) -> Option<String> {
        self.then(String::new)
    }
}
impl<T: AttrValue> AttrValue for Option<T> {
    fn attr_value(self) -> Option<String> {
        self.and_then(AttrValue::attr_value)
    }
}
impl<T: Display + Clone + 'static> AttrValue for Sig<T> {
    fn attr_value(self) -> Option<String> {
        Some(self.get().to_string())
    }
}
impl<T: Display + Clone + 'static> AttrValue for Memo<T> {
    fn attr_value(self) -> Option<String> {
        Some(self.get().to_string())
    }
}
