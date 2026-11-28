# yawn-rsx

The `.rsx` compiler: turns `.rsx` files (real Rust + declarative markup) into Rust that drives the
fine-grained renderer in `yawn::ui`. No proc macro: it's an ordinary build step, so
generated code is a normal file you can read (`.yawn/rsx/`), errors keep their `.rsx`
location, and `yawn dev` runs it before every client build.

## The format
```rust
// src/counter.rsx  ->  module `crate::counter`
use crate::stepper::Stepper;          // everything outside `component` is plain Rust, copied as is

component Counter() {                 // props are fields: `component Card(title: &'static str, children: Children)`
    let count = signal(0i32);         // setup: real Rust, runs once when the component is created
    let double = memo(move || count.get() * 2);
    effect(move || set_title(&format!("Count: {count}")));

    view {
        <p class="count" class:big={count.get() >= 10}>{count}</p>
        <p>"double = {double}"</p>
        <button on:click={move || count.update(|c| *c += 1)}>"+"</button>
        <Stepper step={step} />
        if count.get() >= 10 { <p>"a lot"</p> } else { <p>"keep going"</p> }
        <ul>
            for item in items.get() key item.id { <li>"{item.name}"</li> }
        </ul>
    }
}
```

| construct | meaning |
|---|---|
| `<tag attr="text">` | HTML element; void tags (`input`, `br`, ...) need no closing tag |
| `<Name prop={expr} flag />` | component: a struct literal `Name { prop: expr, flag: true }`, rendered once. A string value goes through `Into`, so `title="x"` fills a `String` or `&str` |
| `"text {expr} {n:03} {{"` | string; `{expr}` interpolates (`format!` specs allowed). Without `{}` it's static |
| `{expr}` | a child: numbers, strings, signals/memos become a text node that follows them; a `View` or `Children` is inserted once |
| `name="x {expr}"`, `name={expr}` | attribute; reactive when it reads signals. `bool` sets/removes a valueless attribute, `Option` `None` removes it |
| `on:event={handler}` | listener; `\|\| ...` or `\|e: Event\| ...`; writes inside are batched; removed with its owner |
| `class:name={cond}` | toggles one class |
| `prop:name={expr}` | sets a DOM property (e.g. an input's `value`) |
| `ref={el}` | fills a `NodeRef` (`let el = node_ref();`) with the element: empty before it exists and after it's gone, with `el.rect()`, `el.focus()`, `el.blur()`, `el.scroll_into_view()` and, in the browser, `el.js_value()` |
| `if cond { } else if cond { } else { }` | conditional block; rebuilt only when the condition flips |
| `if let PAT = value { } else { }` | like `if`, with the pattern's bindings in scope; rebuilt whenever `value` changes (so its type needs `Clone + PartialEq`), not only when the pattern starts or stops matching |
| `for pat in expr key keyexpr { }` | keyed list: rows are built once per key, moved (not rebuilt) on reorder, disposed on removal |
| `style { css }` | plain CSS in the component (after or before `view`), global, written to `/pkg/rsx.css` which the dev server links automatically |
| `server fn name(arg: T) -> R { }` | outside components: runs on the server, the client calls it like an async function (see [Server functions](#server-functions-server-fn)) |
| `name: Class` (prop) | `Class` (`Prop<String>`) takes `class="x"`, `class="a {x}"` and `class={sig.into()}` (a signal or memo); read it as `{class}` in a view and the page follows the signal. `.and("b")` and `.when(|| cond, "c")` join names. `Prop<T>` is the same for any type; a `&'static str` prop takes literals only. Interpolated text props go through `Into`, `{expr}` props don't: write `.into()` |
| `<slot />` | where a component with a `children: Children` prop places what was written between its tags. `<slot name="x" />` places the prop `x: Children`, which the caller fills with `slot="x"` on a child (`<b slot="x">`) or with the contents of `<template slot="x">`; anything without a `slot` goes to `children`. A slot is built each time it's placed, so one inside `if` follows the branch (captured Strings must be signals, as in any branch) |

Not supported (clear diagnostics where it can be detected): `match`/`while`/`loop` in a view
(compute it in setup and write `{value}`), text without quotes, generics on components, a `<style>`
scope (`style { }` is global), `async`.

## Architecture
```text
.rsx source
  -> scan.rs      Rust-aware tokens (strings, chars, lifetimes, comments, delimiters): finds the
                  extent of every Rust fragment and reports unclosed / mismatched delimiters
  -> parser.rs    file = Rust chunks + components; view = nodes (element, text, {expr}, if, for, slot).
                  Every Rust fragment (setup, expressions, conditions, patterns, params) is then
                  validated by `syn`, with errors mapped back to the exact `.rsx` span
  -> ast.rs       nodes refer to the source by byte range, nothing is copied
  -> codegen.rs   struct + `impl Component`; setup copied verbatim; the view becomes straight-line
                  code: create each node once, bind dynamic parts to closures (`bind_attr`,
                  `text_fmt`, `show`, `each`, `listen`, ...). Writes a SourceMap
  -> project.rs   every `src/**/*.rsx` -> `.yawn/rsx/**.rs` (+ `.map`) and `modules.rs`
                  (declares `crate::counter`, `crate::ui::card`); files are only rewritten when
                  their content changes, so cargo stays fresh
```
The generated code only calls `yawn::ui::rt`, which talks to a `Backend` trait (browser DOM,
in-memory tree for tests). Nothing in it is tied to the browser, so the same components can later be
rendered to a string on the server.

**Source maps.** Rust fragments are copied byte for byte, so a rustc error inside `{expr}` or setup
code maps to the same line and column of the `.rsx` file (verbatim runs map column by column; the
glue lines map to the construct they belong to). `yawn dev` rewrites cargo's diagnostics with
these maps, so type errors show up at `src/counter.rsx:47:65`, in the terminal and in the browser.

**Loop variables.** A `for` row closure owns its item; every closure inside the row that uses the item
(handlers, bindings, nested blocks) gets its own `Clone`, so `on:click={move || remove(item.id)}`
just works. Items must be `Clone`.

## Use it
**Entry point.** `src/main.rsx` is the application's root: the component named `App` is what the browser
mounts, and everything outside a component is plain Rust (`use`, `mod helpers;` which finds
`src/helpers.rs`). `yawn dev` / `yawn rsx` generate the crate roots from it into `.yawn/` (`client.rs`: the WASM
library with the `main.rsx` code, the other `.rsx` modules and the mount of `App`; `server.rs`: the
server) together with a copy of your `Cargo.toml` that points `[lib]` and `[[bin]]` at them; cargo is run
on that copy. Your `src/` holds only your own files and your `Cargo.toml` needs nothing beyond the
`yawn-web` dependency. Only `yawn dev` / `yawn rsx` build such a project (a plain `cargo build` finds no
targets). A top-level `mod helpers;` in `main.rsx` is pointed at `src/helpers.rs` (or `src/helpers/mod.rs`);
a file `src/x.rs` with child modules in `src/x/` isn't supported there, use `src/x/mod.rs`. Without a
`main.rsx` the hand-written `src/lib.rs` (`yawn::rsx_modules!();`, `yawn::client!(counter::Counter);`)
and `src/main.rs` keep working as before, with `crate-type = ["cdylib", "rlib"]` in `Cargo.toml`.

**Routes.** `src/routes/` is turned into a route table (Next.js style) by default; see
`crates/yawn-router/README.md`. `[router] folders = false` in `Yawn.toml` turns it off.

`yawn dev` compiles `.rsx` on
every client build; `yawn rsx` only compiles and reports errors. A plain `cargo build` needs one of
them to have run first (the modules file lives in `.yawn/rsx/`).

## Tests
`tests/compile.rs`: codegen shape, diagnostics (message + position), source maps. The sibling crate
`yawn-rsx-tests` compiles real `.rsx` fixtures at build time and renders them natively against the
in-memory document: reactivity, events, conditions, keyed lists (reuse and moves), nested components,
slots, disposal of effects, listeners and nodes.

## API routes (`src/api/`)

Files in `src/api/` become HTTP endpoints, as `pages/api` does in Next.js. They're plain Rust,
compiled for the server only, and need no registration.

| file                     | endpoint             |
|--------------------------|----------------------|
| `api/health.rs`          | `/api/health`        |
| `api/index.rs`           | `/api`               |
| `api/users/[id].rs`      | `/api/users/{id}`    |
| `api/files/[...path].rs` | `/api/files/{*path}` |
| `api/(admin)/stats.rs`   | `/api/stats`         |

A file defines one handler per HTTP method it answers: `pub async fn get`, `post`, `put`, `patch`,
`delete`, `head`, `options`. They're axum handlers (`yawn::server::axum`), so extractors work:

```rust
// src/api/users/[id].rs
use yawn::server::axum::{extract::Path, Json};

pub async fn get(Path(id): Path<String>) -> Json<String> {
    Json(id)
}
```

A file with no handler, a name that isn't a valid route, and two files that make the same route
are errors. The endpoints are merged into the app before `server::configure`, which stays for
middleware, state and anything else that isn't a file.

## Server functions (`server fn`)

A `server fn` in any `.rsx` file runs on the server, and the client calls it like any other async
function:

```rust
// src/todos.rsx  ->  crate::todos::add
server fn add(title: String) -> usize {
    crate::server::db().insert(title)
}

component NewTodo() {
    let count = signal(0);
    view {
        <button on:click={move || spawn(add("milk".into()), move |r| count.set(r.unwrap_or(0)))}>"add"</button>
    }
}
```

The compiler writes two functions with the signature `pub async fn add(title: String) ->
Result<usize, yawn::fns::Error>`: the server's runs the body, the client's posts the arguments as
JSON to `/__yawn/fn/todos/add` and returns the answer. Both return `Result`, so a component that calls
one compiles for SSR too; on the server the call is a direct call and never fails. The server binary
routes every `server fn` of the project.

- Arguments are `name: Type` and, like the result, `serde` types. They come from the browser:
  validate them as you would any request.
- The body is async (`.await` works) and compiled for the server only. Imports it alone needs go
  inside it, or behind `#[cfg(not(target_arch = "wasm32"))]`: the rest of the file is compiled for
  the client as well.
- The server answers only `application/json` requests, which a cross-site form can't send.

Not supported yet: generics, argument patterns (`(a, b): (u8, u8)`), streaming, and defining one in
a plain `.rs` file.
