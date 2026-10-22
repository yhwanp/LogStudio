#!/bin/sh
# Builds these benchmarks against the core of an earlier commit, so a change
# can be measured against its baseline in separate processes. Exports that
# commit's core and workspace files into <dir> (outside this checkout), copies
# this bench crate next to them and builds the requested binaries; they land
# in <dir>/target/release. Only binaries whose core API exists at that commit
# build (each source's header says which run against a baseline).
# Usage: bench/baseline.sh <commit> <dir> <binary>...
set -e
COMMIT=${1:?commit}
DIR=${2:?output dir}
shift 2
[ $# -gt 0 ] || { echo "name the binaries to build" >&2; exit 2; }
ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
[ ! -e "$DIR" ] || { echo "$DIR exists; remove it or pick another" >&2; exit 2; }
mkdir -p "$DIR"
git -C "$ROOT" archive "$COMMIT" core Cargo.toml Cargo.lock | tar -x -C "$DIR"
# The toolchain pin is newer than some commits; without it rustup picks the default.
git -C "$ROOT" archive "$COMMIT" rust-toolchain.toml 2>/dev/null | tar -x -C "$DIR" 2>/dev/null || true
mkdir "$DIR/bench"
cp -R "$ROOT/bench/Cargo.toml" "$ROOT/bench/src" "$DIR/bench/"
sed -i.orig 's/^members = .*/members = ["core","bench"]/' "$DIR/Cargo.toml"
rm "$DIR/Cargo.toml.orig"
for bin in "$@"; do set -- "$@" --bin "$bin"; shift; done
cargo build --release --manifest-path "$DIR/bench/Cargo.toml" "$@"
