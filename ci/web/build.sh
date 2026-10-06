#!/usr/bin/env bash
# Builds the probe with coverage counters and the shim with Istanbul counters into ci/web/site.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
site="$here/site"
rm -rf "$site"
# Cargo and rustup read the probe's own config and toolchain from the working directory.
(cd "$here/probe" && cargo build)
wasm-bindgen --target web --out-dir "$site" --out-name probe \
  "$here/probe/target/wasm32-unknown-unknown/debug/pushups_web_probe.wasm"
# The worker in Rust mode loads this module on its own, so it starts itself as dx's glue does.
printf '\nexport const ready = __wbg_init();\n' >> "$site/probe.js"
cp "$here/index.html" "$site/"
node "$here/instrument.mjs" "$here/../../js/pushups-sw.js" "$site/pushups-sw.js" "$site"/snippets/pushups-*/js/pushups-sw.js
