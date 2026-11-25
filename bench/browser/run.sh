#!/usr/bin/env bash
# Builds the bench app against one snapshot of the framework and measures it.
#   ./run.sh <label> <framework root> <yawn binary> <work dir> [runs]
# The app is copied into <work dir> so the snapshot and the repository stay untouched.
set -euo pipefail
name=$1; root=$(realpath "$2"); yawn=$(realpath "$3"); work=$4; runs=${5:-15}
here=$(cd "$(dirname "$0")" && pwd)
rm -rf "$work/$name"; mkdir -p "$work/$name"
cp -r "$here/app" "$work/$name/app"
sed -i "s#\.\./\.\./\.\./crates#$root/crates#" "$work/$name/app/Cargo.toml"
( cd "$work/$name/app" && "$yawn" build 2>&1 | grep -E "built in|error" )
node "$here/run.mjs" --dist "$work/$name/app/dist" --bin browser-bench --name "$name" --out "$work/$name.json" --runs "$runs"
