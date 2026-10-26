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

# Without a signing identity the harness runs ad-hoc signed and skips the keychain cases, since
# macOS kills a process that claims a keychain access group no provisioning profile grants. With
# PUSHUPS_HARNESS_IDENTITY, PUSHUPS_HARNESS_PROFILE and PUSHUPS_HARNESS_TEAM it runs as an app
# bundle the profile entitles to its access group, keychain cases included.
HARNESS_BUNDLE_ID="${PUSHUPS_HARNESS_BUNDLE_ID:-rs.pushups.ci.harness}"
HARNESS_PLIST="$BUILD/harness-Info.plist"
mkdir -p "$BUILD"
cp "$CI/harness/Info.plist" "$HARNESS_PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $HARNESS_BUNDLE_ID" "$HARNESS_PLIST"
SWIFT_DEFINES=()
if [ -n "${PUSHUPS_HARNESS_IDENTITY:-}" ]; then
    : "${PUSHUPS_HARNESS_PROFILE:?the provisioning profile for $HARNESS_BUNDLE_ID}"
    : "${PUSHUPS_HARNESS_TEAM:?the team identifier of the signing identity}"
    HARNESS_GROUP="$PUSHUPS_HARNESS_TEAM.$HARNESS_BUNDLE_ID"
    /usr/libexec/PlistBuddy -c "Add :PushupsKeychainAccessGroup string $HARNESS_GROUP" "$HARNESS_PLIST"
    /usr/libexec/PlistBuddy -c "Add :CFBundleExecutable string harness" "$HARNESS_PLIST"
    SWIFT_DEFINES=(-D PUSHUPS_KEYCHAIN)
fi

xcrun --sdk macosx swiftc \
    -o "$BUILD/harness" \
    ${SWIFT_DEFINES[@]+"${SWIFT_DEFINES[@]}"} \
    "$CI/harness/main.swift" \
    "$ROOT/ios/PushupsNotificationService.swift" \
    "$BUILD/rust/debug/$LIB" \
    -framework Foundation -framework UserNotifications -framework AppKit \
    -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$HARNESS_PLIST"
if [ -n "${PUSHUPS_HARNESS_IDENTITY:-}" ]; then
    HARNESS_APP="$BUILD/Harness.app"
    mkdir -p "$HARNESS_APP/Contents/MacOS"
    cp "$BUILD/harness" "$HARNESS_APP/Contents/MacOS/harness"
    cp "$HARNESS_PLIST" "$HARNESS_APP/Contents/Info.plist"
    cp "$PUSHUPS_HARNESS_PROFILE" "$HARNESS_APP/Contents/embedded.provisionprofile"
    HARNESS_ENTITLEMENTS="$BUILD/harness.entitlements"
    plutil -create xml1 "$HARNESS_ENTITLEMENTS"
    /usr/libexec/PlistBuddy \
        -c "Add :com.apple.application-identifier string $HARNESS_GROUP" \
        -c "Add :com.apple.developer.team-identifier string $PUSHUPS_HARNESS_TEAM" \
        -c "Add :keychain-access-groups array" \
        -c "Add :keychain-access-groups:0 string $HARNESS_GROUP" \
        "$HARNESS_ENTITLEMENTS"
    codesign --force --sign "$PUSHUPS_HARNESS_IDENTITY" \
        ${PUSHUPS_HARNESS_KEYCHAIN:+--keychain "$PUSHUPS_HARNESS_KEYCHAIN"} \
        --entitlements "$HARNESS_ENTITLEMENTS" "$HARNESS_APP"
    codesign --verify --strict --verbose=2 "$HARNESS_APP"
    "$HARNESS_APP/Contents/MacOS/harness"
else
    codesign --force --sign - "$BUILD/harness"
    "$BUILD/harness"
fi

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
