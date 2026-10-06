#!/usr/bin/env bash
# Turns the counters the harness saved under <out dir> into LCOV files for Codecov and Sonar.
# ci/web/report.sh <out dir> <Rust LCOV file> <shim LCOV file>
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=$(realpath "$1")
rust_lcov=$(realpath -m "$2")
shim_lcov=$(realpath -m "$3")
repo=$(realpath "$here/../..")
tools=$(cd "$here/probe" && echo "$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin")
"$tools/llvm-profdata" merge -sparse "$out"/rust/*.profraw -o "$out/rust.profdata"
"$tools/llvm-cov" export -format=lcov -instr-profile="$out/rust.profdata" \
  "$here/probe/target/wasm32-unknown-unknown/debug/pushups_web_probe.wasm" "$repo/src" > "$rust_lcov"
node "$here/shim-lcov.mjs" "$out/js" "$shim_lcov"
