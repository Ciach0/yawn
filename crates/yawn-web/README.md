# yawn-web

Framework core. One crate, selected by target and feature, so an application depends on it once.

| module | target | what |
|---|---|---|
| `config` | any (feature `config`) | `Yawn.toml`, project root detection, port resolution |
| `protocol` | any | CLI <-> dev server messages (reload / css / error) |
| `server` | native (feature `server`, default) | axum server: HTML shell, `/pkg`, `/assets`, SPA fallback, dev reload channel (SSE) |
| `ui` | any | fine-grained renderer `.rsx` compiles to: `Sig`/`Memo`/`effect` without `&mut Runtime`, `show`, keyed `each`, `Children`, a `Backend` trait and an in-memory `ui::mock` document for native tests |
| `web` | wasm32 | browser `Backend` (DOM, listener lifetime) and the `client!` entry macro |

The CLI uses `yawn-web` with `default-features = false, features = ["config"]`, so it doesn't
compile axum or tokio.

## An application

With `.rsx` the application is `src/main.rsx` (`component App() { view { ... } }`); `yawn dev` generates the
server and client crate roots from it (see `crates/yawn-rsx/README.md`). Written by hand, the two
halves are:
```rust
// src/main.rs  (native: the server)
fn main() -> std::process::ExitCode {
    yawn::server::App::new("counter").start()
}
```
```rust
// src/lib.rs  (the client)
yawn::rsx_modules!();                 // modules generated from src/*.rsx
#[cfg(target_arch = "wasm32")]
yawn::client!(counter::Counter);      // mounts the component into #app
```
See `examples/counter` and `crates/yawn-rsx/README.md`.

## `ui`: how a component runs
A component is built once. Its view creates every node once and binds the dynamic parts (text,
attribute, class, `if` branch, list) to effects, so a changed signal updates exactly what reads it:
no virtual DOM, and the component function never re-runs. Components, branches and list rows own
what they create (effects, memos, listeners, nodes): it's disposed when they go away.
`Sig::get()` and friends use an ambient runtime (one audited `unsafe` module, `ui/ambient.rs`); they
panic outside render/effects/handlers, and inside `Sig::update`/`Sig::with` closures (create signals
before `update`, don't read signals inside it).

## Port
`yawn dev --port N` (passed as `YAWN_PORT`) > `PORT` > `[server] port` in `Yawn.toml` > 3000.
A bad `PORT` is an error, not a silent fallback. `cargo run` without the CLI resolves the same way.

## Extension points
* `App::merge(axum::Router)`: your own routes (server functions, SSR routes) take precedence.
* `Document::render`: the shell is a pure function, ready to be produced by SSR.
* `protocol::Control`: new dev-time messages (e.g. a patch for `.rsx` modules) are one more variant;
  the browser side is the small script in `src/dev_client.js`.
