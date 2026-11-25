# Editor support for `.rsx`

| part | what |
|---|---|
| `crates/yawn-lsp` | language server (stdio). Shared by every editor |
| `editors/tree-sitter-rsx` | grammar + queries: highlighting of markup, Rust and CSS (injected), brackets, indents |
| `editors/zed-rsx` | Zed extension: language, queries, starts `yawn-lsp` |

## How it works
* **Rust**: `yawn-lsp` starts rust-analyzer and shows it every `.rsx` file as the Rust module the
  compiler generates from it (`.yawn/rsx/*.rs`, kept in sync with the unsaved buffer). Completion,
  hover, go to definition, references, signature help and diagnostics go through the source map, so
  they appear at the right place in the `.rsx` file. Type errors in `{expressions}` and setup code
  are real rust-analyzer diagnostics. Syntax errors of the format itself are reported by
  `yawn-rsx`.
* **HTML**: `vscode-html-languageservice` (the engine of VS Code) completes tags and attributes and
  gives hover (MDN text). It sees a copy of the file in which everything that isn't markup is blanked
  out (`mask.rs`), so positions are identical. Yawn adds what it can't know: project components
  with their props (from `component Name(prop: Type)`, also hover and go to definition), `on:` events
  (the DOM's `onclick`... names from the HTML data), `class:` / `prop:` / `ref`, `if` / `for` snippets.
* **Components**: choosing a component from the completion list also adds its `use crate::...;` line
  (below the file's last `use`), and "Import `crate::x::Card`" is offered as a quick fix on a tag whose
  component isn't imported. Go to definition, references and rename work on component names across
  all `.rsx` files and the project's `.rs` files; document and workspace symbols list components.
* **Rust actions**: code actions (quick fixes, refactors, organize imports), inlay hints (types of `let`s),
  rename of local names, semantic tokens and rust-analyzer's signature help work in setup code and
  `{expressions}`. Formatting runs `rustfmt` on setup code and plain Rust items (markup is left alone).
  An edit that would land in generated code (not something you wrote) is dropped, never half-applied.
* **From `.rs` to `.rsx`**: the server is also attached to Rust files; for them it only adds jumps
  (definition, references...) whose target is a generated module, translated to the `.rsx` file. The
  editor's own rust-analyzer still lists the generated file as well. It starts rust-analyzer only when
  a `Yawn.toml` project is found below the opened folder, so other Rust projects are unaffected.
* **Typing**: `>` after `<div>` inserts `</div>` (not for void tags or `/>`), and Enter between `<div>` and
  `</div>` opens an indented line (`textDocument/onTypeFormatting`).
* **CSS**: `style { ... }` in a component is plain CSS (global, collected into `/pkg/rsx.css`).
  `vscode-css-languageservice` completes, shows hover and lints it (diagnostics).
  Both services run in a small Node helper (`crates/yawn-lsp/web`, bundled into `dist/web.cjs` and
  embedded in the binary). Node.js (or Bun) must be installed; `YAWN_NODE` overrides the path. Without
  it only Rust and the Yawn-specific completions work (the trace log says so).
* rust-analyzer is found via `YAWN_RUST_ANALYZER`, then `rust-analyzer` on PATH, then the copy
  Zed downloaded. Without it HTML and CSS still work. Every `Yawn.toml` project below the opened
  folder is handed to it (`linkedProjects`), so opening a monorepo root or the repository root works,
  not only the project folder itself.

## Install
```sh
cargo install --path crates/yawn-lsp          # puts `yawn-lsp` on PATH
editors/zed-rsx/scripts/install-zed.sh           # prepares the grammar for Zed
# Zed: command palette -> "zed: install dev extension" -> the directory the script prints
```
`install-zed.sh` is needed because Zed fetches grammars from a git repository at a commit: it
copies the grammar into a throwaway repository and the extension into a directory next to it, both
under `~/.local/share/yawn/`. Nothing in this checkout changes.

Open the project once with `yawn dev` or `yawn rsx` before editing: they create
`.yawn/rsx/` (generated modules), which rust-analyzer needs (`.yawn/` is git-ignored; it lives
outside `target/` because rust-analyzer doesn't follow modules into `target/`).

## Other editors
* Any LSP client: run `yawn-lsp` over stdio for files matching `*.rsx` (language id `rsx`).
* Neovim / Helix: use `editors/tree-sitter-rsx` (`grammar.js`, `src/`, `queries/`) for highlighting
  and the command above for the server.

## Limits (first iteration)
* Semantic tokens are used by Zed only if enabled (`"semantic_tokens": "combined"` in the language settings).
  Markup has completion and hover but no rename, formatting or code actions (except component imports).
* HTML has no diagnostics (the markup isn't validated); CSS is global, not scoped to the component.
* Zed itself wasn't driven in this environment: the extension compiles (`cargo check`) and the
  grammar and server are tested standalone.

## When something doesn't show up
`~/.cache/yawn-lsp/lsp.log` (truncated on every start) lists the roots and projects handed to
rust-analyzer, every request with the context it was classified in, and what rust-analyzer answered. After
reinstalling the server restart it in Zed (`editor: restart language server`).

## Developing the server
`cargo install --path crates/yawn-lsp --force`. The HTML/CSS bundle is checked in
(`crates/yawn-lsp/web/dist/web.cjs`); after changing `web/web.mjs` or its dependencies run
`npm install && npm run build` in `crates/yawn-lsp/web`.

## Completion colours in Zed
Zed colours a completion label by parsing it with the buffer's grammar, which for `.rsx` sees no Rust.
`zed-rsx/src/lib.rs` (`label_for_completion`) therefore builds the label from spans with explicit
highlight names (function, type, variable...), and `highlights.scm` registers those names (Zed only
accepts names that occur in the language's query). After changing either, rebuild the dev extension.
