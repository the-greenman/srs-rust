#!/usr/bin/env bash
# Scaled-corpus benchmark (srs-rust#1194): one command reproduces the x1 / x10 table.
# Usage: scripts/bench-scale.sh [source-archive.srs] [scale]
# Defaults: the vendored pinned muSrs archive, scale 10. See docs/benchmarking.md.
set -euo pipefail
cd "$(dirname "$0")/.."

SRC="${1:-crates/srs-repository/tests/fixtures/discovery-eval/musrs-pinned.srs}"
SCALE="${2:-10}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cargo build --release -q --bin srs
cargo build --release -q -p srs-repository --example scale_bench
BENCH=target/release/examples/scale_bench

target/release/srs archive unpack "$SRC" --target "$WORK/x1" >/dev/null
"$BENCH" gen "$WORK/x1" "$WORK/x$SCALE" "$SCALE"
"$BENCH" bench "$WORK/x1" "$WORK/x$SCALE"
