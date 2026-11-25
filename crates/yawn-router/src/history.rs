//! Where the current URL comes from and how it changes: the browser's history, or a list in
//! memory for tests and non-browser targets.

use crate::location::Location;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A source of locations
pub trait History {
    /// The location now
    fn current(&self) -> Location;
    /// Navigates to `url` and adds an entry
    fn push(&self, url: &str);
    /// Navigates to `url` in place of the current entry
    fn replace(&self, url: &str);
    /// One entry back, if there's one
    fn back(&self);
    /// One entry forward, if there's one
    fn forward(&self);
    /// `callback` runs after the location changed
    fn subscribe(&self, callback: Box<dyn Fn()>);
}

/// Entries kept in memory
pub struct MemoryHistory {
    entries: RefCell<Vec<String>>,
    index: Cell<usize>,
    subscribers: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl MemoryHistory {
    /// A history with one entry, `start` (for example `"/users/3"`)
    pub fn new(start: &str) -> MemoryHistory {
        MemoryHistory {
            entries: RefCell::new(vec![start.to_string()]),
            index: Cell::new(0),
            subscribers: RefCell::default(),
        }
    }

    fn notify(&self) {
        let subs: Vec<_> = self.subscribers.borrow().clone();
        for s in subs {
            s();
        }
    }
}

impl History for MemoryHistory {
    fn current(&self) -> Location {
        Location::parse(&self.entries.borrow()[self.index.get()])
    }

    fn push(&self, url: &str) {
        let at = self.index.get() + 1;
        let mut e = self.entries.borrow_mut();
        e.truncate(at);
        e.push(url.to_string());
        self.index.set(at);
        drop(e);
        self.notify();
    }

    fn replace(&self, url: &str) {
        self.entries.borrow_mut()[self.index.get()] = url.to_string();
        self.notify();
    }

    fn back(&self) {
        if self.index.get() > 0 {
            self.index.set(self.index.get() - 1);
            self.notify();
        }
    }

    fn forward(&self) {
        if self.index.get() + 1 < self.entries.borrow().len() {
            self.index.set(self.index.get() + 1);
            self.notify();
        }
    }

    fn subscribe(&self, callback: Box<dyn Fn()>) {
        self.subscribers.borrow_mut().push(Rc::from(callback));
    }
}

/// The browser's session history (`pushState`, `popstate`)
#[cfg(target_arch = "wasm32")]
pub struct BrowserHistory {
    subscribers: Rc<RefCell<Vec<Rc<dyn Fn()>>>>,
}

#[cfg(target_arch = "wasm32")]
impl BrowserHistory {
    /// Listens to `popstate` (back and forward buttons) for the life of the page
    pub fn new() -> BrowserHistory {
        use wasm_bindgen::closure::Closure;
        let subscribers: Rc<RefCell<Vec<Rc<dyn Fn()>>>> = Rc::default();
        let subs = subscribers.clone();
        let on_pop = Closure::<dyn FnMut(yawn::dom::Event)>::new(move |_| {
            // the listener outlives the page's client when a hot reload replaces it; the old one has nothing left to tell
            if !yawn::ui::is_mounted() {
                return;
            }
            let list: Vec<_> = subs.borrow().clone();
            list.iter().for_each(|s| s());
        });
        if let Some(w) = yawn::dom::window() {
            let _ = w.target().add_event_listener("popstate", &on_pop);
        }
        // lives as long as the page
        on_pop.forget();
        BrowserHistory { subscribers }
    }

    fn notify(&self) {
        let list: Vec<_> = self.subscribers.borrow().clone();
        list.iter().for_each(|s| s());
    }

    fn history() -> Option<yawn::dom::History> {
        yawn::dom::window()?.history().ok()
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for BrowserHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_arch = "wasm32")]
impl History for BrowserHistory {
    fn current(&self) -> Location {
        let l = yawn::dom::window().expect("no window").location();
        Location {
            path: l.pathname().unwrap_or_else(|_| "/".into()),
            query: l.search().unwrap_or_default().trim_start_matches('?').to_string(),
            hash: l.hash().unwrap_or_default().trim_start_matches('#').to_string(),
        }
    }

    fn push(&self, url: &str) {
        if let Some(h) = Self::history() {
            let _ = h.push_state(&wasm_bindgen::JsValue::NULL, "", url);
        }
        self.notify();
    }

    fn replace(&self, url: &str) {
        if let Some(h) = Self::history() {
            let _ = h.replace_state(&wasm_bindgen::JsValue::NULL, "", url);
        }
        self.notify();
    }

    fn back(&self) {
        // the `popstate` that follows notifies
        if let Some(h) = Self::history() {
            let _ = h.back();
        }
    }

    fn forward(&self) {
        if let Some(h) = Self::history() {
            let _ = h.forward();
        }
    }

    fn subscribe(&self, callback: Box<dyn Fn()>) {
        self.subscribers.borrow_mut().push(Rc::from(callback));
    }
}

/// `url` as the browser sees it under `base` (`/pl`): `/guilds` becomes `/pl/guilds`, `/` becomes
/// `/pl`
pub fn with_base(base: &str, url: &str) -> String {
    if base.is_empty() || !url.starts_with('/') || url.starts_with("//") {
        return url.to_string();
    }
    let path_end = url.find(['?', '#']).unwrap_or(url.len());
    let (path, rest) = url.split_at(path_end);
    if path == base || path.strip_prefix(base).is_some_and(|r| r.starts_with('/')) {
        return url.to_string();
    }
    if path == "/" { format!("{base}{rest}") } else { format!("{base}{path}{rest}") }
}

/// A history whose URLs live under a base path: the router sees `/guilds` while the address bar
/// shows `/pl/guilds`
pub struct BaseHistory {
    inner: Rc<dyn History>,
    base: String,
}

impl BaseHistory {
    /// Wraps `inner`, showing every URL under `base`
    pub fn new(inner: Rc<dyn History>, base: &str) -> BaseHistory {
        BaseHistory { inner, base: base.trim_end_matches('/').to_string() }
    }
}

impl History for BaseHistory {
    fn current(&self) -> Location {
        let mut l = self.inner.current();
        if let Some(rest) = l.path.strip_prefix(&self.base) {
            if rest.is_empty() {
                l.path = "/".into();
            } else if rest.starts_with('/') {
                l.path = rest.to_string();
            }
        }
        l
    }
    fn push(&self, url: &str) {
        self.inner.push(&with_base(&self.base, url));
    }
    fn replace(&self, url: &str) {
        self.inner.replace(&with_base(&self.base, url));
    }
    fn back(&self) {
        self.inner.back();
    }
    fn forward(&self) {
        self.inner.forward();
    }
    fn subscribe(&self, callback: Box<dyn Fn()>) {
        self.inner.subscribe(callback);
    }
}

pub(crate) fn default_history() -> Rc<dyn History> {
    #[cfg(target_arch = "wasm32")]
    {
        Rc::new(BrowserHistory::new())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // while the server renders, the URL of the request being rendered
        let url = yawn::ui::ssr::request().map(|r| r.url).unwrap_or_else(|| "/".to_string());
        Rc::new(MemoryHistory::new(&url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_back_forward_and_replace() {
        let h = MemoryHistory::new("/");
        let seen = Rc::new(Cell::new(0));
        let s = seen.clone();
        h.subscribe(Box::new(move || s.set(s.get() + 1)));
        h.push("/a");
        h.push("/b?x=1");
        assert_eq!(h.current().path, "/b");
        h.back();
        assert_eq!(h.current().path, "/a");
        h.replace("/c");
        h.forward();
        assert_eq!(h.current().url(), "/b?x=1");
        h.forward();
        assert_eq!(h.current().path, "/b");
        h.back();
        h.push("/d");
        h.forward();
        assert_eq!(h.current().path, "/d", "pushing drops the forward entries");
        assert_eq!(seen.get(), 7);
    }

    #[test]
    fn base_history_hides_the_prefix_from_the_router() {
        let inner = Rc::new(MemoryHistory::new("/pl/guilds?x=1"));
        let h = BaseHistory::new(inner.clone(), "/pl");
        assert_eq!(h.current().url(), "/guilds?x=1");
        h.push("/about");
        assert_eq!(inner.current().path, "/pl/about");
        assert_eq!(h.current().path, "/about");
        h.push("/");
        assert_eq!(inner.current().path, "/pl");
        assert_eq!(h.current().path, "/");
        h.push("/pl/x");
        assert_eq!(inner.current().path, "/pl/x");
        let other = BaseHistory::new(Rc::new(MemoryHistory::new("/plx")), "/pl");
        assert_eq!(other.current().path, "/plx");
    }

    #[test]
    fn with_base_cases() {
        assert_eq!(with_base("/pl", "/a?x=1#h"), "/pl/a?x=1#h");
        assert_eq!(with_base("/pl", "/?x=1"), "/pl?x=1");
        assert_eq!(with_base("/pl", "/pl/a"), "/pl/a");
        assert_eq!(with_base("/pl", "/plx"), "/pl/plx");
        assert_eq!(with_base("/pl", "https://a.b"), "https://a.b");
        assert_eq!(with_base("", "/a"), "/a");
    }
}
