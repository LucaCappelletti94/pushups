#!/usr/bin/env bash
# Compares a signed notification service extension's `PushupsNotificationFiltering` declaration with its signed filtering entitlement.
#
# Usage: ios/check-filtering.sh <NotificationService.appex>
set -euo pipefail

appex=${1:?usage: check-filtering.sh <NotificationService.appex>}
codesign --verify --strict "$appex"

entitlements=$(mktemp)
trap 'rm -f "$entitlements"' EXIT
codesign -d --entitlements - --xml "$appex" > "$entitlements" 2>/dev/null

declared=$(/usr/libexec/PlistBuddy -c 'Print :PushupsNotificationFiltering' "$appex/Info.plist" 2>/dev/null) || declared=false
entitled=$(/usr/libexec/PlistBuddy -c 'Print :com.apple.developer.usernotifications.filtering' "$entitlements" 2>/dev/null) || entitled=false

case "$declared/$entitled" in
  false/false)
    echo "check-filtering: no declaration and no entitlement, so Rendering::NoAlert keeps the original alert"
    ;;
  true/true)
    echo "check-filtering: declared and entitled. Apple's approval of the entitlement, apns-push-type alert and suppression on a device still need checking"
    ;;
  true/false)
    echo "check-filtering: PushupsNotificationFiltering is declared but the signature lacks com.apple.developer.usernotifications.filtering, so iOS would show alerts the app asks to suppress" >&2
    exit 1
    ;;
  false/true)
    echo "check-filtering: the signature carries com.apple.developer.usernotifications.filtering but PushupsNotificationFiltering is not declared, so Rendering::NoAlert keeps the original alert" >&2
    exit 1
    ;;
  *)
    echo "check-filtering: unexpected values, declaration '$declared' and entitlement '$entitled'" >&2
    exit 2
    ;;
esac
