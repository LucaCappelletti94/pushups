#!/usr/bin/env bash
# Builds the named example, winit or tauri, with a fresh VAPID key pair, builds the probe's send
# binary, and runs ci/linux/example.sh in the UnifiedPush test bed. Needs Docker and, for tauri,
# the GTK and WebKitGTK development packages.
set -euo pipefail

example=${1:?usage: example-host.sh winit|tauri}
case "$example" in winit | tauri) ;; *) exit 2 ;; esac

ci=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$ci/../.." && pwd)
mkdir -p "$root/target"
out=$(mktemp -d "$root/target/linux-example.XXXXXX")
trap 'rm -rf "$out"' EXIT

# A fresh VAPID key pair for the app's server, the private PEM and the base64url public point.
openssl ecparam -name prime256v1 -genkey -noout -out "$out/vapid.pem"
openssl ec -in "$out/vapid.pem" -pubout -outform DER 2>/dev/null | tail -c 65 \
  | base64 -w0 | tr '+/' '-_' | tr -d '=' > "$out/vapid.b64url"

(cd "$ci/probe" && cargo build --quiet --release --bin send)
(
  cd "$root/examples/$example"
  PUSHUPS_VAPID_PUBLIC_KEY="$(cat "$out/vapid.b64url")" cargo build --quiet --release
)

# The host's user, whose uid need not be the container's, must be able to delete what lands here.
chmod -R a+rwX "$out"
PUSHUPS_TESTBED_OUT="$out" "$ci/testbed.sh" bash /work/ci/linux/example.sh "$example"
