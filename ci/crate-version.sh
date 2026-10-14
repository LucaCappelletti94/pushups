#!/usr/bin/env bash
# Prints the pushups crate's version, the fact the release scripts and the version handshake compare.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo pkgid -p pushups | sed 's/.*[#@]//'
