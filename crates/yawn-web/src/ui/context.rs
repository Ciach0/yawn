use super::ambient;
use super::signals::{Sig, on_cleanup, signal_always};
use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

thread_local! {
    static CONTEXT: RefCell<HashMap<TypeId, Vec<(u64, Box<dyn Any>)>>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(0) };
}

/// Makes `value` available to [`use_context`] for as long as the component calling this lives
pub fn provide<T: Clone + 'static>(value: T) {
    ambient::with(|rt| rt.provide_context(value.clone()));
    let at = NEXT.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    CONTEXT
        .with(|c| c.borrow_mut().entry(TypeId::of::<T>()).or_default().push((at, Box::new(value))));
    on_cleanup(move || {
        CONTEXT.with(|c| {
            if let Some(stack) = c.borrow_mut().get_mut(&TypeId::of::<T>()) {
                stack.retain(|(other, _)| *other != at);
            }
        })
    });
}

/// The value of type `T` the nearest component above provided, if any
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    if let Some(found) =
        ambient::is_active().then(|| ambient::with(|rt| rt.context::<T>())).flatten()
    {
        return Some(found);
    }
    CONTEXT.with(|c| {
        c.borrow()
            .get(&TypeId::of::<T>())
            .and_then(|s| s.last())
            .and_then(|(_, v)| v.downcast_ref::<T>())
            .cloned()
    })
}

/// Runs `future` in the browser and hands what it returns to `then`, inside the component, effect
/// or event handler that called this
pub fn spawn<T: 'static>(
    future: impl Future<Output = T> + 'static,
    then: impl FnOnce(T) + 'static,
) {
    let alive = Rc::new(Cell::new(true));
    // an effect that runs again is the same owner, still alive, but the answer to its earlier run is stale
    if ambient::with(|rt| rt.has_owner() && !rt.in_handler()) {
        let alive = alive.clone();
        on_cleanup(move || alive.set(false));
    }
    let owner = ambient::capture_owner();
    #[cfg(target_arch = "wasm32")]
    crate::dom::spawn_local(async move {
        let out = future.await;
        if alive.get() {
            ambient::run_in_owner(owner, || then(out));
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (future, then, alive, owner);
    }
}

/// Asks, on the server, for what a [`resource`] further down the page will need, without waiting
/// until the page gets there
pub fn prefetch<F>(name: &str, server: impl FnOnce() -> F)
where
    F: Future<Output = Option<String>> + 'static,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = super::ssr::fetched(name, || Box::pin(server()));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (name, server);
    }
}

#[cfg(target_arch = "wasm32")]
type Waiter = (Vec<String>, Rc<RefCell<Option<Box<dyn FnOnce()>>>>);

#[cfg(target_arch = "wasm32")]
type Live = Vec<Sig<Option<String>>>;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PRELOADED: RefCell<HashMap<String, (Option<Option<String>>, i64)>> = RefCell::new(HashMap::new());
    static WAITERS: RefCell<Vec<Waiter>> = const { RefCell::new(Vec::new()) };
    static LIVE: RefCell<HashMap<String, Live>> = RefCell::new(HashMap::new());
    static URLS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static REFRESHING: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
}

#[cfg(target_arch = "wasm32")]
const FRESH_MS: i64 = 3_000;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PUSHED: Cell<bool> = const { Cell::new(false) };
}

/// Tells the cache that something else (a [`stream`](super::stream::stream_with_state) the server
/// pushes changes through) will call [`invalidate`] whenever the data changes, so a downloaded text
/// stays current until then and not for a few seconds only
pub fn keep_until_invalidated(on: bool) {
    #[cfg(target_arch = "wasm32")]
    PUSHED.with(|p| p.set(on));
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on;
}

#[cfg(target_arch = "wasm32")]
fn current(age: i64) -> bool {
    PUSHED.with(|p| p.get()) || age < FRESH_MS
}

/// Forgets every downloaded text, so the next navigation that needs one fetches it again and waits
/// for it
pub fn invalidate() {
    #[cfg(target_arch = "wasm32")]
    {
        PRELOADED.with(|p| p.borrow_mut().retain(|_, (state, _)| state.is_none()));
        let shown: Vec<String> = ambient::run_if_mounted(|| {
            LIVE.with(|l| {
                l.borrow()
                    .iter()
                    .filter(|(_, sigs)| sigs.iter().any(|s| s.is_alive()))
                    .map(|(name, _)| name.clone())
                    .collect()
            })
        })
        .unwrap_or_default();
        for name in shown {
            if let Some(url) = URLS.with(|u| u.borrow().get(&name).cloned()) {
                refresh(&name, &url);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn fetch_text(url: String) -> Option<String> {
    let req = crate::dom::FetchRequest {
        url: &url,
        method: "GET",
        headers: &[],
        body: &[],
        timeout_ms: 30000,
    };
    match crate::dom::fetch_bytes(req).await {
        Ok(res) if (200..300).contains(&res.status) => String::from_utf8(res.body).ok(),
        _ => None,
    }
}

#[cfg(target_arch = "wasm32")]
fn remember(name: &str, text: String) {
    let at = crate::browser::platform::now_millis();
    PRELOADED.with(|p| p.borrow_mut().insert(name.to_string(), (Some(Some(text)), at)));
}

#[cfg(target_arch = "wasm32")]
fn cached(name: &str) -> Option<(String, i64)> {
    let now = crate::browser::platform::now_millis();
    PRELOADED.with(|p| match p.borrow().get(name) {
        Some((Some(Some(text)), at)) if current(now - at) => Some((text.clone(), now - at)),
        _ => None,
    })
}

#[cfg(target_arch = "wasm32")]
fn is_loading(name: &str) -> bool {
    PRELOADED.with(|p| matches!(p.borrow().get(name), Some((None, _))))
}

/// Whether any of the texts called `names` is being downloaded for the first time, so a page that
/// reads it has nothing to start from
pub fn loading(names: &[String]) -> bool {
    #[cfg(target_arch = "wasm32")]
    return names.iter().any(|n| is_loading(n));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = names;
        false
    }
}

#[cfg(target_arch = "wasm32")]
fn settle() {
    let ready: Vec<Waiter> = WAITERS.with(|w| {
        let mut all = w.borrow_mut();
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut *all)
            .into_iter()
            .partition(|(names, _)| !names.iter().any(|n| is_loading(n)));
        *all = waiting;
        ready
    });
    for (_, slot) in ready {
        let then = slot.borrow_mut().take();
        if let Some(then) = then {
            ambient::run_if_mounted(|| super::signals::batch(then));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn publish(name: &str, text: &str) {
    let sigs: Live = LIVE.with(|l| l.borrow().get(name).cloned().unwrap_or_default());
    if sigs.is_empty() {
        return;
    }
    ambient::run_if_mounted(|| {
        super::signals::batch(|| {
            for sig in sigs.iter().filter(|s| s.is_alive()) {
                if sig.peek().as_deref() != Some(text) {
                    sig.try_update(|v| *v = Some(text.to_string()));
                }
            }
        });
        LIVE.with(|l| {
            if let Some(all) = l.borrow_mut().get_mut(name) {
                all.retain(|s| s.is_alive());
            }
        });
    });
}

#[cfg(target_arch = "wasm32")]
fn refresh(name: &str, url: &str) {
    if REFRESHING.with(|r| !r.borrow_mut().insert(name.to_string())) {
        return;
    }
    let (name, url) = (name.to_string(), url.to_string());
    crate::dom::spawn_local(async move {
        let text = fetch_text(url).await;
        REFRESHING.with(|r| r.borrow_mut().remove(&name));
        if let Some(text) = text {
            remember(&name, text.clone());
            publish(&name, &text);
        } else {
            PRELOADED.with(|p| p.borrow_mut().remove(&name));
        }
        settle();
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn preload_url(name: &str, url: &str) {
    if crate::web::boot(name).is_some() {
        return;
    }
    URLS.with(|u| u.borrow_mut().insert(name.to_string(), url.to_string()));
    let now = crate::browser::platform::now_millis();
    let seen =
        PRELOADED.with(|p| p.borrow().get(name).map(|(state, at)| (state.clone(), now - at)));
    match seen {
        Some((None, _)) => return,
        Some((Some(Some(_)), age)) if current(age) => return,
        _ => {}
    }
    if REFRESHING.with(|r| r.borrow().contains(name)) {
        PRELOADED.with(|p| p.borrow_mut().insert(name.to_string(), (None, now)));
        return;
    }
    PRELOADED.with(|p| p.borrow_mut().insert(name.to_string(), (None, now)));
    let (name, url) = (name.to_string(), url.to_string());
    crate::dom::spawn_local(async move {
        let text = fetch_text(url).await;
        match text {
            Some(text) => {
                remember(&name, text.clone());
                publish(&name, &text);
            }
            None => {
                let at = crate::browser::platform::now_millis();
                PRELOADED.with(|p| p.borrow_mut().insert(name, (Some(None), at)));
            }
        }
        settle();
    });
}

/// Whether a download that a navigation would have to wait for is still going in the browser: one
/// of a text nothing was kept of
pub fn preloading() -> bool {
    #[cfg(target_arch = "wasm32")]
    return PRELOADED.with(|p| p.borrow().values().any(|(state, _)| state.is_none()));
    #[cfg(not(target_arch = "wasm32"))]
    false
}

/// Calls `then` once none of the texts called `names` is still being downloaded for the first time,
/// or after `timeout_ms` if that takes longer, whichever comes first; at once when none is
pub fn when_loaded(names: &[String], timeout_ms: i32, then: impl FnOnce() + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (names, timeout_ms);
        then();
    }
    #[cfg(target_arch = "wasm32")]
    {
        if !names.iter().any(|n| is_loading(n)) {
            then();
            return;
        }
        let slot: Rc<RefCell<Option<Box<dyn FnOnce()>>>> =
            Rc::new(RefCell::new(Some(Box::new(then))));
        WAITERS.with(|w| w.borrow_mut().push((names.to_vec(), slot.clone())));
        super::timer::after(timeout_ms, move || {
            let then = slot.borrow_mut().take();
            if let Some(then) = then {
                then();
            }
        });
    }
}

/// Like [`when_loaded`], for everything being downloaded now
pub fn when_preloaded(timeout_ms: i32, then: impl FnOnce() + 'static) {
    #[cfg(target_arch = "wasm32")]
    {
        let names: Vec<String> = PRELOADED.with(|p| {
            p.borrow()
                .iter()
                .filter(|(_, (state, _))| state.is_none())
                .map(|(n, _)| n.clone())
                .collect()
        });
        when_loaded(&names, timeout_ms, then);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = timeout_ms;
        then();
    }
}

/// Text the page needs at startup, with `None` until it's there
pub fn resource<F>(name: &str, url: &str, server: impl FnOnce() -> F) -> Sig<Option<String>>
where
    F: Future<Output = Option<String>> + 'static,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        let text = super::ssr::fetched(name, || Box::pin(server())).flatten();
        if text.is_some() {
            super::head::boot(name, url);
        }
        signal_always(text)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = server;
        let boot = crate::web::boot(name);
        let kept = cached(name);
        let booted = boot.or_else(|| kept.map(|(text, _)| text));
        let missing = booted.is_none();
        // never carried over a rebuild (see `carry`): resources of one kind share a place in the source
        let out = signal_always(booted);
        URLS.with(|u| u.borrow_mut().insert(name.to_string(), url.to_string()));
        LIVE.with(|l| l.borrow_mut().entry(name.to_string()).or_default().push(out));
        if missing {
            let (url, name) = (url.to_string(), name.to_string());
            spawn(fetch_text(url), move |text| {
                if let Some(text) = text {
                    remember(&name, text.clone());
                    out.set(Some(text));
                }
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::mock;
    use crate::ui::rt::Frag;
    use crate::ui::{Component, View};

    struct Page;
    impl Component for Page {
        fn render(self) -> View {
            Frag::new().finish()
        }
    }

    #[test]
    fn spawn_works_from_an_event_handler_that_belongs_to_no_component() {
        let _app = mock::mount(Page);
        crate::ui::run(|| spawn(async {}, |_| {}));
    }

    thread_local! {
        static SEEN: Cell<(u32, bool)> = const { Cell::new((0, false)) };
    }

    struct Button;
    impl Component for Button {
        fn render(self) -> View {
            let mut f = Frag::new();
            let b = crate::ui::rt::element("button");
            crate::ui::rt::listen(b, "click", move |_: crate::ui::Event| {
                spawn(async {}, |_| {});
                let owned = ambient::with(|rt| rt.has_owner() && rt.in_handler());
                SEEN.with(|s| s.set((s.get().0 + 1, owned)));
            });
            crate::ui::rt::add(&mut f, b);
            f.finish()
        }
    }

    #[test]
    fn an_event_handler_runs_inside_the_component_that_registered_it() {
        let app = mock::mount(Button);
        app.dispatch(app.find("button"), "click", crate::ui::Event::default());
        assert_eq!(SEEN.with(|s| s.get()), (1, true));
    }

    #[test]
    fn a_value_provided_by_a_component_is_found_by_what_it_builds_later() {
        struct Parent;
        impl Component for Parent {
            fn render(self) -> View {
                provide(7u32);
                Frag::new().finish()
            }
        }
        let _app = mock::mount(Parent);
        assert_eq!(use_context::<u32>(), Some(7));
    }
}
