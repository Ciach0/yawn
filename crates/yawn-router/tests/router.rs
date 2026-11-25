//! The router against the in-memory document: navigation, nesting, params, links.

use std::rc::Rc;
use yawn::ui::mock::{self, MockApp};
use yawn::ui::rt::{self, Frag};
use yawn::ui::{Children, Component, Event, View};
use yawn_router::{
    History, Link, MemoryHistory, Outlet, Routes, render_with, use_location, use_navigate,
    use_params, use_query,
};

fn tag(name: &str, text: &str) -> View {
    let mut f = Frag::new();
    let e = rt::element(name);
    rt::add(e, rt::text(text));
    rt::add(&mut f, e);
    f.finish()
}

#[derive(Default)]
struct Shell;
impl Component for Shell {
    fn render(self) -> View {
        let mut f = Frag::new();
        let div = rt::element("div");
        rt::attr(div, "class", "shell");
        rt::add(div, tag("h1", "shell"));
        rt::add(div, Outlet::default().render());
        rt::add(&mut f, div);
        f.finish()
    }
}

#[derive(Default)]
struct Home;
impl Component for Home {
    fn render(self) -> View {
        tag("p", "home")
    }
}

#[derive(Default)]
struct User;
impl Component for User {
    fn render(self) -> View {
        let params = use_params();
        let query = use_query();
        let mut f = Frag::new();
        let p = rt::element("p");
        rt::attr(p, "class", "user");
        rt::add(
            p,
            rt::text_fmt(move || {
                format!("user {} tab {}", params.get("id"), query.get("tab").unwrap_or_default())
            }),
        );
        rt::add(&mut f, p);
        f.finish()
    }
}

#[derive(Default)]
struct Missing;
impl Component for Missing {
    fn render(self) -> View {
        let loc = use_location();
        let mut f = Frag::new();
        let p = rt::element("p");
        rt::add(p, rt::text_fmt(move || format!("no page {}", loc.get().path)));
        rt::add(&mut f, p);
        f.finish()
    }
}

struct Nav;
impl Component for Nav {
    fn render(self) -> View {
        let mut f = Frag::new();
        let nav = rt::element("nav");
        for (href, label) in [("/", "home"), ("/users/1", "one"), ("https://example.com", "out")] {
            let l = Link { href: href.into(), children: Children::new(move || tag("span", label)) };
            rt::add(nav, l.render());
        }
        rt::add(&mut f, nav);
        f.finish()
    }
}

fn routes() -> Routes {
    Routes::new()
        .layout::<Shell>("", |r| r.at::<Home>("/").at::<User>("/users/:id").not_found::<Missing>())
}

struct App(Rc<MemoryHistory>);
impl Component for App {
    fn render(self) -> View {
        let mut f = Frag::new();
        // the router has to exist before anything that uses it's built; the order in the
        // document is still nav first
        let router = render_with(routes(), self.0.clone());
        let nav = Nav.render();
        rt::add(&mut f, nav);
        rt::add(&mut f, router);
        f.finish()
    }
}

fn start(url: &str) -> (MockApp, Rc<MemoryHistory>) {
    let h = Rc::new(MemoryHistory::new(url));
    (mock::mount(App(h.clone())), h)
}

#[test]
fn renders_the_route_for_the_start_url_inside_its_layout() {
    let (app, _) = start("/");
    assert_eq!(app.text(app.find(".shell")), "shellhome");
    let (app, _) = start("/users/42?tab=posts");
    assert_eq!(app.text(app.find(".user")), "user 42 tab posts");
    let (app, _) = start("/zzz");
    assert_eq!(app.text(app.find(".shell")), "shellno page /zzz");
}

#[test]
fn navigation_swaps_only_the_outlet_and_keeps_the_layout() {
    let (app, h) = start("/");
    let shell_h1 = app.find(".shell h1");
    h.push("/users/1");
    assert_eq!(app.text(app.find(".shell")), "shelluser 1 tab ");
    assert_eq!(app.find(".shell h1"), shell_h1, "the layout is not rebuilt");
    h.back();
    assert_eq!(app.text(app.find(".shell")), "shellhome");
}

#[test]
fn a_page_that_stays_mounted_follows_param_changes() {
    let (app, h) = start("/users/1");
    let p = app.find(".user");
    h.push("/users/2");
    assert_eq!(app.find(".user"), p, "same route: the node is reused");
    assert_eq!(app.text(p), "user 2 tab ");
    h.replace("/users/3?tab=x");
    assert_eq!(app.text(p), "user 3 tab x");
}

#[test]
fn links_navigate_prevent_the_browser_and_mark_the_active_one() {
    let (app, h) = start("/");
    let links = app.find_all("nav a");
    assert_eq!(links.len(), 3);
    assert_eq!(app.attr(links[1], "href").as_deref(), Some("/users/1"));
    assert_eq!(app.attr(links[0], "class").as_deref(), Some("active"));
    let ev = Event::default();
    app.dispatch(links[1], "click", ev.clone());
    assert!(ev.default_prevented());
    assert_eq!(h.current().path, "/users/1");
    assert_eq!(app.text(app.find(".shell")), "shelluser 1 tab ");
    assert_eq!(app.attr(links[1], "class").as_deref(), Some("active"));
    assert_ne!(app.attr(links[0], "class").as_deref(), Some("active"));
    // new-tab clicks and other sites are left alone
    let mut ev = Event::default();
    ev.modified = true;
    app.dispatch(links[0], "click", ev.clone());
    assert!(!ev.default_prevented());
    assert_eq!(h.current().path, "/users/1");
    let ev = Event::default();
    app.dispatch(links[2], "click", ev.clone());
    assert!(!ev.default_prevented(), "external links have no handler");
}

#[test]
fn navigating_from_code_uses_the_navigator() {
    struct Go;
    impl Component for Go {
        fn render(self) -> View {
            let nav = use_navigate();
            let mut f = Frag::new();
            let b = rt::element("button");
            rt::listen(b, "click", move || nav.push("/users/9"));
            rt::add(&mut f, b);
            f.finish()
        }
    }
    struct Page(Rc<MemoryHistory>);
    impl Component for Page {
        fn render(self) -> View {
            let mut f = Frag::new();
            rt::add(
                &mut f,
                render_with(Routes::new().at::<Home>("/").at::<User>("/users/:id"), self.0.clone()),
            );
            // components built after the router can use it too
            rt::add(&mut f, Go.render());
            f.finish()
        }
    }
    let h = Rc::new(MemoryHistory::new("/"));
    let app = mock::mount(Page(h.clone()));
    app.click(app.find("button"));
    assert_eq!(app.text(app.find("p.user")), "user 9 tab ");
}

#[test]
fn disposing_the_router_leaves_no_listeners_or_nodes_behind() {
    let (app, h) = start("/users/1");
    let before = app.live_nodes();
    h.push("/");
    h.push("/users/5");
    h.push("/");
    assert!(app.live_nodes() <= before, "{} > {before}", app.live_nodes());
}

#[derive(Default)]
struct Rebuilding;
impl Component for Rebuilding {
    fn render(self) -> View {
        let params = use_params();
        let mut f = Frag::new();
        let div = rt::element("div");
        rt::attr(div, "class", "guild");
        rt::add(div, rt::switch(move || params.find("id"), |_| Outlet::default().render()));
        rt::add(&mut f, div);
        f.finish()
    }
}

struct Guilds(Rc<MemoryHistory>);
impl Component for Guilds {
    fn render(self) -> View {
        render_with(
            Routes::new().layout::<Rebuilding>("/g/:id", |r| r.at::<Home>("/")),
            self.0.clone(),
        )
    }
}

#[test]
fn an_outlet_built_after_its_layout_still_shows_the_route_below() {
    let h = Rc::new(MemoryHistory::new("/g/1"));
    let app = mock::mount(Guilds(h.clone()));
    assert_eq!(app.text(app.find(".guild")), "home");
    h.push("/g/2");
    assert_eq!(app.text(app.find(".guild")), "home", "the rebuilt outlet is not the layout again");
    assert_eq!(app.find_all(".guild").len(), 1);
}

thread_local! {
    static ASKED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn ask_for_the_layout(_: &yawn_router::Target) -> Vec<yawn::data::Source> {
    ASKED.with(|a| a.borrow_mut().push("layout".into()));
    Vec::new()
}

fn ask_for_the_user(target: &yawn_router::Target) -> Vec<yawn::data::Source> {
    let id = target.params.get("id");
    let tab = target.query.get("tab").unwrap_or_default();
    ASKED.with(|a| a.borrow_mut().push(format!("user {id} {tab}")));
    Vec::new()
}

struct Preloaded(Rc<MemoryHistory>);
impl Component for Preloaded {
    fn render(self) -> View {
        let routes = Routes::new()
            .layout::<Shell>("", |r| {
                r.at::<Home>("/").at::<User>("/users/:id").preload(ask_for_the_user)
            })
            .preload(ask_for_the_layout);
        render_with(routes, self.0.clone())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_matched_routes_are_asked_what_they_need_before_they_are_built() {
    ASKED.with(|a| a.borrow_mut().clear());
    let _app = mock::mount(Preloaded(Rc::new(MemoryHistory::new("/users/7?tab=posts"))));
    assert_eq!(ASKED.with(|a| a.borrow().clone()), ["layout", "user 7 posts"]);
    ASKED.with(|a| a.borrow_mut().clear());
    let _app = mock::mount(Preloaded(Rc::new(MemoryHistory::new("/"))));
    assert_eq!(ASKED.with(|a| a.borrow().clone()), ["layout"], "the page of / has none");
}

struct Taken(Rc<MemoryHistory>);
impl Component for Taken {
    fn render(self) -> View {
        let routes = Routes::new().at::<TakeIt>("/g/:id/:module");
        render_with(routes, self.0.clone())
    }
}

#[derive(Default)]
struct TakeIt;
impl Component for TakeIt {
    fn render(self) -> View {
        let params = use_params();
        let [id, module] = params.take(["id", "module"]);
        assert_eq!(params.find("nope"), None);
        assert_eq!(params.get("nope"), "");
        tag("p", &format!("{id}/{module}"))
    }
}

#[test]
fn parameters_are_text_and_can_be_taken_apart() {
    let app = mock::mount(Taken(Rc::new(MemoryHistory::new("/g/7/settings"))));
    assert_eq!(app.text(app.find("p")), "7/settings");
}

struct Valued(Rc<MemoryHistory>);
impl Component for Valued {
    fn render(self) -> View {
        let routes =
            Routes::new().layout::<ValuedShell>("/g/:id", |r| r.at::<ValuedPage>("/:module"));
        render_with(routes, self.0.clone())
    }
}

#[derive(Default)]
struct ValuedShell;
impl Component for ValuedShell {
    fn render(self) -> View {
        let (id,) = use_params().values();
        let mut f = Frag::new();
        rt::add(&mut f, tag("h2", &id));
        rt::add(&mut f, Outlet::default().render());
        f.finish()
    }
}

#[derive(Default)]
struct ValuedPage;
impl Component for ValuedPage {
    fn render(self) -> View {
        let (id, module) = use_params().values();
        tag("p", &format!("{id}-{module}"))
    }
}

#[test]
fn parameters_come_in_the_order_of_the_address() {
    let app = mock::mount(Valued(Rc::new(MemoryHistory::new("/g/7/settings"))));
    assert_eq!(app.text(app.find("h2")), "7");
    assert_eq!(app.text(app.find("p")), "7-settings");
}
