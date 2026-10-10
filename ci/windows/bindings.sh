#!/bin/sh
# Regenerates src/windows/bindings.rs from the Windows App SDK metadata.
# The .winmd files are fetched from the NuGet packages, so no metadata is committed.
# Run from the repository root: ci/windows/bindings.sh
set -eu

# The pinned metapackage release. ci/windows/gen pins the generator version.
METAPACKAGE_VERSION=2.5.1

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fetch() {
    curl -fsSL --max-time 600 -o "$3" \
        "https://api.nuget.org/v3-flatcontainer/$1/$2/$1.$2.nupkg"
}

fetch microsoft.windowsappsdk "$METAPACKAGE_VERSION" "$work/metapackage.nupkg"
unzip -q "$work/metapackage.nupkg" -d "$work/metapackage"

# The Foundation split package that holds the .winmd metadata, per the metapackage nuspec.
foundation=$(sed -n 's/.*<dependency id="Microsoft.WindowsAppSDK.Foundation" version="\([0-9.]*\)".*/\1/p' \
    "$work/metapackage/Microsoft.WindowsAppSDK.nuspec")
[ -n "$foundation" ] || { echo "no Microsoft.WindowsAppSDK.Foundation dependency in the nuspec" >&2; exit 1; }
fetch microsoft.windowsappsdk.foundation "$foundation" "$work/foundation.nupkg"
unzip -q "$work/foundation.nupkg" -d "$work/foundation"

push="$work/foundation/metadata/Microsoft.Windows.PushNotifications.winmd"
lifecycle="$work/foundation/metadata/Microsoft.Windows.AppLifecycle.winmd"
[ -f "$push" ] || { echo "missing $push" >&2; exit 1; }
[ -f "$lifecycle" ] || { echo "missing $lifecycle" >&2; exit 1; }

cargo run --quiet --manifest-path ci/windows/gen/Cargo.toml -- "$push" "$lifecycle"
# Formatted as `cargo fmt` would, so the committed file is exactly what this script writes.
rustfmt --edition 2024 src/windows/bindings.rs
echo "wrote src/windows/bindings.rs (Microsoft.WindowsAppSDK $METAPACKAGE_VERSION, Foundation $foundation)"
