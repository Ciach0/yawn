# Contributing

Bug reports and pull requests are welcome. For anything larger than a fix, open an issue first so
we can agree on the direction.

## Setup

You need Rust (see `rust-version` in `Cargo.toml`) and the `wasm32-unknown-unknown` target. `yawn`
downloads the `wasm-bindgen` CLI matching the example's `Cargo.lock` when the one on `PATH` doesn't.

```sh
rustup target add wasm32-unknown-unknown
cargo install --path crates/yawn-cli          # the `yawn` command from this checkout
```

## Before a pull request

CI runs these; running them locally saves a round trip.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p yawn-web -p yawn-kit --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

`bench/` and `examples/` are separate workspaces; build them from their own directories.

## Notes

* A change to the reactivity API needs a semantics test in `crates/yawn-reactivity/tests/`.
* A change to `.rsx` syntax needs a fixture in `crates/yawn-rsx-tests/fixtures/` and the grammar in
  `editors/tree-sitter-rsx` updated.
* `crates/yawn-lsp/web/dist/web.cjs` is generated (`npm install && npm run build` in
  `crates/yawn-lsp/web`) and committed, because the language server embeds it.
* Keep user-visible text and docs plain and short. Comments explain why, not what.

By contributing you agree that your work is licensed under the same terms as the project
(MIT OR Apache-2.0).
