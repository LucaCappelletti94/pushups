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
    -framework Foundation -framework UserNotifications -framework AppKit \
    -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$CI/harness/Info.plist"
# The protected keychain refuses a process without the entitlement for the access group the harness declares.
HARNESS_ENTITLEMENTS="$BUILD/harness.entitlements"
plutil -create xml1 "$HARNESS_ENTITLEMENTS"
/usr/libexec/PlistBuddy -c 'Add :keychain-access-groups array' "$HARNESS_ENTITLEMENTS"
/usr/libexec/PlistBuddy -c 'Add :keychain-access-groups:0 string rs.pushups.ci.harness' "$HARNESS_ENTITLEMENTS"
codesign --force --sign - --entitlements "$HARNESS_ENTITLEMENTS" "$BUILD/harness"
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

# The release checker against an iOS extension signed with and without the entitlement and the declaration.
FIXTURE="$BUILD/check-filtering"
ENTITLEMENTS="$FIXTURE/filtering.entitlements"
mkdir -p "$FIXTURE"
plutil -create xml1 "$ENTITLEMENTS"
/usr/libexec/PlistBuddy -c 'Add :com.apple.developer.usernotifications.filtering bool true' "$ENTITLEMENTS"
check_row() {
    local declared="$1" entitled="$2" expected="$3"
    local bundle="$FIXTURE/$declared-$entitled/NotificationService.appex"
    mkdir -p "$(dirname "$bundle")"
    cp -R "$BUILD/iphoneos/NotificationService.appex" "$bundle"
    if [ "$declared" = declared ]; then
        plutil -insert PushupsNotificationFiltering -bool true "$bundle/Info.plist"
    fi
    if [ "$entitled" = entitled ]; then
        codesign --force --sign - --entitlements "$ENTITLEMENTS" "$bundle"
    else
        codesign --force --sign - "$bundle"
    fi
    local status=0
    "$ROOT/ios/check-filtering.sh" "$bundle" || status=$?
    if [ "$status" != "$expected" ]; then
        echo "check-filtering: $declared and $entitled exited $status, expected $expected" >&2
        exit 1
    fi
}
check_row undeclared unentitled 0
check_row declared entitled 0
check_row declared unentitled 1
check_row undeclared entitled 1
