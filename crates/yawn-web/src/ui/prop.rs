use super::signals::{Memo, Sig};
use std::fmt;
use std::rc::Rc;

/// A value read when it is needed, so it can follow signals
pub struct Prop<T: 'static>(Rc<dyn Fn() -> T>);

/// A prop holding CSS class names
pub type Class = Prop<String>;

impl<T: 'static> Clone for Prop<T> {
    fn clone(&self) -> Self {
        Prop(self.0.clone())
    }
}

impl<T: 'static> Prop<T> {
    /// A prop computed on every read; reading signals inside makes the reader reactive
    pub fn new(f: impl Fn() -> T + 'static) -> Self {
        Prop(Rc::new(f))
    }

    pub fn get(&self) -> T {
        (self.0)()
    }
}

impl<T: Clone + 'static> From<T> for Prop<T> {
    fn from(value: T) -> Self {
        Prop::new(move || value.clone())
    }
}

impl From<&'static str> for Prop<String> {
    fn from(value: &'static str) -> Self {
        Prop::new(move || value.to_string())
    }
}

impl<T: Clone + 'static> From<Sig<T>> for Prop<T> {
    fn from(s: Sig<T>) -> Self {
        Prop::new(move || s.get())
    }
}

impl<T: Clone + 'static> From<Memo<T>> for Prop<T> {
    fn from(m: Memo<T>) -> Self {
        Prop::new(move || m.get())
    }
}

impl Default for Prop<String> {
    fn default() -> Self {
        Prop::new(String::new)
    }
}

impl<T: fmt::Display + 'static> fmt::Display for Prop<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Prop<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Prop({:?})", self.get())
    }
}

impl Prop<String> {
    /// These class names, then `other`'s, separated by a space (an empty side adds nothing)
    pub fn and(self, other: impl Into<Prop<String>>) -> Prop<String> {
        let other = other.into();
        Prop::new(move || {
            let (a, b) = (self.get(), other.get());
            match (a.is_empty(), b.is_empty()) {
                (true, _) => b,
                (_, true) => a,
                _ => format!("{a} {b}"),
            }
        })
    }

    /// These class names, plus `class` while `condition` is true
    pub fn when(
        self,
        condition: impl Fn() -> bool + 'static,
        class: impl Into<Prop<String>>,
    ) -> Prop<String> {
        let class = class.into();
        self.and(Prop::new(move || if condition() { class.get() } else { String::new() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Component, Sig, View, mock, rt, signal};

    #[test]
    fn joins_class_names_and_skips_empty_ones() {
        let c =
            Class::from("a").and("b").and(String::new()).when(|| false, "off").when(|| true, "on");
        assert_eq!(c.get(), "a b on");
        assert_eq!(Class::default().and("x").get(), "x");
    }

    struct Box_(Class);
    impl Component for Box_ {
        fn render(self) -> View {
            let div = rt::element("div");
            let class = self.0;
            rt::bind_attr(div, "class", move || format!("{class} fixed"));
            div.into()
        }
    }

    thread_local! {
        static SIG: std::cell::Cell<Option<Sig<String>>> = const { std::cell::Cell::new(None) };
    }

    // builds the signal where signals may be made: while the app renders
    struct WithSignal;
    impl Component for WithSignal {
        fn render(self) -> View {
            let s = signal("one".to_string());
            SIG.with(|c| c.set(Some(s)));
            Box_(s.into()).render()
        }
    }

    #[test]
    fn a_signal_prop_keeps_the_attribute_up_to_date() {
        let app = mock::mount(WithSignal);
        assert_eq!(app.html(), "<div class=\"one fixed\"></div>");
        let s = SIG.with(|c| c.get()).unwrap();
        crate::ui::run(|| s.set("two".to_string()));
        assert_eq!(app.html(), "<div class=\"two fixed\"></div>");
        let literal = mock::mount(Box_("lit".into()));
        assert_eq!(literal.html(), "<div class=\"lit fixed\"></div>");
    }
}
