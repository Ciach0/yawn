//! Client-side routing for Yawn applications.
//!
//! ```ignore
//! // main.rsx
//! use yawn_router::{Router, Routes};
//! component App() {
//!     view { <Router routes={Routes::new().layout::<Shell>("", |r| r.at::<Home>("/").at::<About>("/about"))} /> }
//! }
//! ```
//!
//! A [`Router`] watches the URL and renders the matching routes. A layout puts the route
//! below it where it renders `<Outlet />`; [`Link`] navigates without a page load. Inside
//! components, [`use_params`], [`use_location`] and [`use_navigate`] give access to the router.
//!
//! With `src/routes/` and folder routing on (the default; `[router] folders = false` in
//! `Yawn.toml` turns it off), `yawn` builds the table from the file tree and defines `crate::routes::table()`.

#![warn(missing_docs)]
#![allow(clippy::type_complexity)]

mod history;
mod location;
mod router;
mod routes;

#[cfg(target_arch = "wasm32")]
pub use history::BrowserHistory;
pub use history::{BaseHistory, History, MemoryHistory, with_base};
pub use location::Location;
pub use router::{
    FromParams, Link, LocationHandle, Navigator, Outlet, Params, PreloadFn, Query, Router, Target,
    render_with, use_location, use_navigate, use_navigating, use_params, use_query,
};
pub use routes::{Pattern, Routes, Seg};
