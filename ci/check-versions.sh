#!/usr/bin/env bash
# Fails unless the Android module's `pushups.module_version` equals the crate's version, which the
# version handshake compares at runtime. With a tag argument (`v<version>`), the tag must equal it too.
#
# Usage: ci/check-versions.sh [tag]
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
crate=$(cd "$root" && cargo pkgid -p pushups | sed 's/.*[#@]//')
module=$(sed -n '/pushups.module_version/{n;s/.*android:value="\([^"]*\)".*/\1/p}' "$root/android/src/main/AndroidManifest.xml")
echo "crate $crate, Android module $module${1:+, tag $1}"
[ "$module" = "$crate" ] || { echo "android/src/main/AndroidManifest.xml says $module, Cargo.toml says $crate" >&2; exit 1; }
[ -z "${1:-}" ] || [ "$1" = "v$crate" ] || { echo "tag $1 is not v$crate" >&2; exit 1; }
