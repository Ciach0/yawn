#!/usr/bin/env bash
# Prepares a copy of the extension for "zed: install dev extension". Zed fetches grammars from a
# git repository at a commit, so this puts the grammar into a throwaway git repository and a copy
# of the extension that points at it. Nothing in this checkout changes.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
grammar="$here/../tree-sitter-rsx"
data="${XDG_DATA_HOME:-$HOME/.local/share}/yawn"
repo="$data/tree-sitter-rsx"
ext="$data/zed-rsx"

rm -rf "$repo" "$ext" && mkdir -p "$repo" "$ext"
cp -r "$grammar/grammar.js" "$grammar/tree-sitter.json" "$grammar/package.json" "$grammar/src" "$grammar/queries" "$repo/"
git -C "$repo" init -q
git -C "$repo" add -A
git -C "$repo" -c user.name=yawn -c user.email=yawn@localhost commit -q -m "tree-sitter-rsx"
rev="$(git -C "$repo" rev-parse HEAD)"

cp -r "$here/extension.toml" "$here/Cargo.toml" "$here/Cargo.lock" "$here/src" "$here/languages" "$ext/"
sed -i "/^path = /d" "$ext/extension.toml"
sed -i "/^\[grammars.rsx\]/,\$ s|^repository = .*|repository = \"file://$repo\"|; s|^commit = .*|commit = \"$rev\"|" "$ext/extension.toml"

echo "grammar: $repo @ $rev"
echo "next: cargo install --path crates/yawn-lsp   (once), then in Zed: zed: install dev extension -> $ext"
