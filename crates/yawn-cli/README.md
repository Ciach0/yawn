# yawn-cli

The `yawn` binary: `dev`, `rsx`, `build`, `new` (also `init`), `upgrade` and `info`. Output is a TUI on an interactive
terminal, plain text lines with `--plain` (also automatic under CI, pipes and `TERM=dumb`), or JSON
lines with `--json`.

## Layers
`event`, `state` (pure data and logic) -> `pipeline` (one bounded queue) -> presenters `tui` /
`output` -> `session`, `commands`, `cli` (wiring). A future `dev` or `build` implements
`pipeline::EventSource` and returns a `session::Session`.

## Commands
`dev` (build, serve, rebuild on change), `rsx` (compile `.rsx` and print errors only), `new` (creates a project; `init` is the same), `upgrade` (installs the latest release with `cargo binstall`, or `cargo install` without it; `--check` only looks), `info`.
`dev/rsx.rs` is the `.rsx` step of a client build and the rustc-to-`.rsx` location remapping.

## TUI keys
`r` reload, `o` open the page, `v` verbose, `t` trace, `c` clear, `/` more details, `Ctrl-C` quit.

## Not verified here
Unit tests cover state, keys, rendering (ratatui `TestBackend`, many sizes), and output formats. The interactive terminal lifecycle (raw mode, restore on quit/signal/panic) and idle CPU
were designed for but not exercised in a real terminal in this session.
