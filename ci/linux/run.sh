#!/usr/bin/env bash
# Builds the Linux probe with coverage, runs ci/linux/scenario.sh in the UnifiedPush test bed, and
# writes target/linux-rust.lcov. Needs Docker, rustup's llvm-tools and openssl.
set -euo pipefail

ci=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$ci/../.." && pwd)
# shellcheck source=ci/vapid.sh
. "$ci/../vapid.sh"
mkdir -p "$root/target"
out=$(mktemp -d "$root/target/linux-run.XXXXXX")
trap 'rm -rf "$out"' EXIT

# An explicit target keeps the coverage flags off build scripts, which would leave profiles behind.
target=x86_64-unknown-linux-gnu
(
  cd "$ci/probe"
  RUSTFLAGS="-Cinstrument-coverage" cargo build --quiet --target "$target"
)

# A fresh VAPID key pair for the app's server, as the private PEM and the raw public point in base64url.
vapid_generate "$out"
vapid_public_b64url "$out/vapid.pem" > "$out/vapid.b64url"
chmod -R a+rwX "$out"

PUSHUPS_TESTBED_OUT="$out" "$ci/testbed.sh" bash /work/ci/linux/scenario.sh

tools=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin
"$tools/llvm-profdata" merge -sparse "$out"/profraw/*.profraw -o "$out/pushups.profdata"
"$tools/llvm-cov" export -format=lcov -instr-profile "$out/pushups.profdata" \
  "$ci/probe/target/$target/debug/pushups-linux-probe" "$root/src" \
  | sed "s|^SF:$root/|SF:|" > "$root/target/linux-rust.lcov"
