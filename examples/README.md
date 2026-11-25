# examples/

| project | shows |
|---|---|
| `counter/` | the smallest `.rsx` app: a signal and two buttons |

```sh
cargo build -p yawn-cli                      # once, from the repo root
cd examples/counter
../../target/debug/yawn dev                  # http://127.0.0.1:3000, or --port 4000
```

Needs the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`) and a
`wasm-bindgen` CLI matching the project's `wasm-bindgen` version (`cargo install wasm-bindgen-cli
--version 0.2.129`; `yawn dev` names the exact command if they differ).

Try it: edit `assets/style.css` (applies without reload) or `src/main.rsx` (compiles to Rust,
rebuilds, the page reloads). Break `src/main.rsx` on purpose, for instance with unquoted text in
the view, to see the error with its `.rsx` location in the terminal and in the browser.
