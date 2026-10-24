#!/usr/bin/env bash
# The Android `Context` must not outlive the push's callback or leave its thread.
set -euo pipefail

target=$1
here=$(cd "$(dirname "$0")" && pwd)
export CARGO_TARGET_DIR=$here/target

check() { cargo check --quiet --target "$target" --manifest-path "$here/Cargo.toml" "$@"; }

check
echo "contract: an unsafe-free app compiles"

reject() {
  local feature=$1 expected=$2 log
  if log=$(check --features "$feature" 2>&1); then
    echo "contract: $feature compiled" >&2
    exit 1
  fi
  if ! grep -qF "$expected" <<<"$log"; then
    echo "contract: $feature failed without '$expected'" >&2
    echo "$log" >&2
    exit 1
  fi
  echo "contract: $feature is rejected"
}

reject keep-context "lifetime may not live long enough"
reject send-context "cannot be shared between threads safely"
reject return-local "lifetime may not live long enough"
