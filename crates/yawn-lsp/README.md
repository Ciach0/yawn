# yawn-lsp

Language server for `.rsx` files (stdio). Rust goes to rust-analyzer through the `.rsx` source map;
HTML and CSS come from `vscode-html-languageservice` / `vscode-css-languageservice` (Node helper in
`web/`); components, props and directives are added here. Setup, limits, troubleshooting:
`editors/README.md`.

Environment: `YAWN_RUST_ANALYZER`, `YAWN_NODE`. Trace: `~/.cache/yawn-lsp/lsp.log`.
