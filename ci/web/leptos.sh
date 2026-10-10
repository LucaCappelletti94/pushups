#!/usr/bin/env bash
# Builds examples/leptos with cargo-leptos in both worker modes, and checks each in Chrome with
# real pushes, served by the app's own server, which serves both workers from routes.
# Run from anywhere after `npm ci` in ci/web.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
app="$here/../../examples/leptos"
origin=http://127.0.0.1:3000
vapid="${RUNNER_TEMP:-$(mktemp -d)}/leptos-vapid.json"
key=$(node "$here/example.mjs" keys "$vapid")
server=
trap '[ -n "$server" ] && kill "$server" 2>/dev/null' EXIT

# check <PUSHUPS_WEB_WORKER> <worker pattern> [example.mjs flags]
check() {
  (cd "$app" && PUSHUPS_WEB_WORKER="$1" PUSHUPS_VAPID_PUBLIC_KEY="$key" cargo leptos build --release)
  (cd "$app" && LEPTOS_OUTPUT_NAME=pushups-leptos-example LEPTOS_SITE_ROOT=target/site \
    LEPTOS_SITE_PKG_DIR=pkg LEPTOS_SITE_ADDR=127.0.0.1:3000 exec ./target/release/pushups-leptos-example) &
  server=$!
  for _ in $(seq 60); do
    curl -fs -o /dev/null "$origin/" && break
    sleep 0.5
  done
  # The route serves `pushups::SERVICE_WORKER`, the crate's worker unchanged.
  curl -fs "$origin/pushups-sw.js" | cmp - "$here/../../js/pushups-sw.js"
  node "$here/example.mjs" check "$origin" "$vapid" "${@:2}"
  kill "$server"
  wait "$server" 2>/dev/null || true
  server=
}

check rust '/sw\.js\?pushups=rust' --rust
check static '/pushups-sw\.js\?pushups=static'
