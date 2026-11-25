pub(crate) mod ambient;
pub(crate) mod backend;
pub(crate) mod context;
pub mod head;
pub(crate) mod hydrate;
pub(crate) mod keep;
pub mod loader;
pub mod mock;
mod node_ref;
pub mod presence;
mod prop;
pub mod rt;
pub(crate) mod signals;
pub mod ssr;
pub(crate) mod stream;
pub(crate) mod template;
pub(crate) mod timer;
mod view;

pub use ambient::{is_active, is_mounted, run};
pub use backend::{Backend, Event, FileRef, Handler, ListenerId, NodeId, Rect, RectSource};
pub use keep::{carry, enable_carry};
pub use node_ref::{Element, NodeRef, element, node_ref};
pub use presence::presence;
pub use prop::{Class, Prop};
pub use rt::mount;
pub use signals::{
    GlobalSig, Memo, Sig, batch, effect, memo, node_count, on_cleanup, signal, signal_always,
    untracked,
};
pub use view::{AttrValue, Child, Children, Frag, IntoChild, Parent, View};

/// A component: props are the fields of the type, `render` builds its nodes once
pub trait Component {
    /// Builds the component's nodes
    fn render(self) -> View;
}

pub fn set_title(title: &str) {
    rt::with_backend(|b| b.set_title(title));
}

/// Imported by every generated module
pub mod prelude {
    pub use super::{Children, Class, Component, Event, Memo, NodeRef, Prop, Sig, View};
    pub use super::{
        batch, effect, memo, node_count, node_ref, on_cleanup, set_title, signal, signal_always,
        untracked,
    };
}
