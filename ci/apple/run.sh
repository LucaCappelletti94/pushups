#!/usr/bin/env bash
# Exercises the native adapter and links the consumer's Rust entry points into iOS extensions.
set -euo pipefail

cd "$(dirname "$0")/../../"
ROOT="$(pwd -P)"
CI="$(cd "$(dirname "$0")" && pwd -P)"
LIB="libpushups_apple_probe.a"
PROBE="$CI/probe/Cargo.toml"

BUILD="$ROOT/target/apple-run"
rm -rf "$BUILD"
trap 'rm -rf "$BUILD"' EXIT

cargo +stable test -p pushups --lib rendering:: --target-dir "$BUILD/rust"
cargo +stable build --manifest-path "$PROBE" --target-dir "$BUILD/rust"
cargo +stable build --manifest-path "$PROBE" --target aarch64-apple-ios-sim --target-dir "$BUILD/rust"
cargo +stable build --manifest-path "$PROBE" --target aarch64-apple-ios --target-dir "$BUILD/rust"

xcrun --sdk macosx swiftc \
    -o "$BUILD/harness" \
    "$CI/harness/main.swift" \
    "$ROOT/ios/PushupsNotificationService.swift" \
    "$BUILD/rust/debug/$LIB" \
    -framework Foundation -framework UserNotifications -framework AppKit
"$BUILD/harness"

build_extension() {
    local sdk="$1" swift_target="$2" rust_target="$3"
    local bundle="$BUILD/$sdk/NotificationService.appex"
    mkdir -p "$bundle"
    xcrun --sdk "$sdk" swiftc \
        -sdk "$(xcrun --sdk "$sdk" --show-sdk-path)" \
        -parse-as-library -application-extension \
        -module-name PushupsAppex \
        -target "$swift_target" \
        -Xlinker -e -Xlinker _NSExtensionMain \
        -o "$bundle/NotificationService" \
        "$ROOT/ios/PushupsNotificationService.swift" \
        "$BUILD/rust/$rust_target/debug/$LIB" \
        -framework Foundation -framework UserNotifications -framework UIKit
    cp "$CI/appex/Info.plist" "$bundle/Info.plist"
    codesign --force --sign - "$bundle"
    codesign --verify --strict --verbose=2 "$bundle"
    otool -hv "$bundle/NotificationService" | grep EXECUTE
    xcrun vtool -show-build "$bundle/NotificationService"
    for symbol in create receive expired deallocated; do
        nm -gU "$bundle/NotificationService" | grep " ___pushups_notification_service_$symbol\$"
    done
}

build_extension iphonesimulator arm64-apple-ios16.0-simulator aarch64-apple-ios-sim
build_extension iphoneos arm64-apple-ios16.0 aarch64-apple-ios
