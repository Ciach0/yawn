# yawn-router

Client-side routing for Yawn: nested routes, `<Outlet />`, `<Link />`, browser history.

Two ways to define routes; use either.

## Folders (default)

With `src/routes/` the table is built from the file tree, as in Next.js, and defined as `table()` in the
`routes` module, which `main.rsx` (the crate root) sees by name; other files import it with `use crate::routes;`. Turn it off with `[router] folders = false` in `Yawn.toml`. `yawn` adds the
`yawn-router` dependency itself (same source as your `yawn-web`).

| file                        | route                                     |
|-----------------------------|-------------------------------------------|
| `routes/index.rsx`          | `/`                                       |
| `routes/about.rsx`          | `/about`                                  |
| `routes/users/index.rsx`    | `/users`                                  |
| `routes/users/[id].rsx`     | `/users/:id`                              |
| `routes/docs/[...path].rsx` | `/docs/*path` (everything below `/docs`)  |
| `routes/layout.rsx`         | wraps every route in its folder and below |
| `routes/not_found.rsx`      | the page for paths nothing else matches   |
| `routes/(group)/x.rsx`      | `/x`: a folder that adds no URL segment   |

The component of a route file is named after the file in PascalCase (`index.rsx` is `Index`, `[id].rsx` is `Id`, `layout.rsx` is `Layout`) and takes no props; other components may stand beside it. A layout renders the route
below it with `<Outlet />`. Static beats `:param` beats `*splat`, whatever the order.

```rust
// src/main.rsx
use yawn_router::Router;
component App() {
    view { <Router routes={routes::table()} /> }
}
```

## In code

```rust
Routes::new().layout::<Shell>("", |r| r
    .at::<Home>("/")
    .at::<User>("/users/:id")
    .not_found::<Missing>())
```

## Under a path prefix

`routes::table().base("/pl")` serves the whole table under `/pl` (a language, or an app mounted in
a subpath): routes are still written and matched as `/guilds`, the address bar shows `/pl/guilds`,
and `Link` and `use_navigate()` add the prefix themselves. Compute the prefix before building the
router (it's a plain string, so it can come from the request when the page is rendered on the
server).

## Using the router

- `<Link href="/about">"About"</Link>` navigates without a page load. Other sites, `mailto:` and
  Ctrl/Cmd/Shift-clicks are left to the browser. The class `active` is set while it points at the
  current path.
- `use_params().get("id")`, `use_query().get("tab")` and `use_location().get()` are reactive: a page
  that stays mounted while `/users/1` becomes `/users/2` sees the new value (the page is rebuilt only
  when a *different* route matches).
- `use_navigate()` gives `push`, `replace`, `back`, `forward`.
- Hooks work in components built after the `Router` (a layout, a page, anything inside).

The dev server (and `yawn::server`) answers every path that isn't an asset with the app's HTML, so
deep links and reloads work. `Router` uses `pushState`/`popstate`; `MemoryHistory` with `render_with`
runs the same router in tests against `yawn::ui::mock`.

Not there yet: server-side data loading, redirects, route guards, scroll restoration, optional
props for `Link` (`class`, replace).
