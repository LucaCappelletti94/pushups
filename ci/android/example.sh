#!/usr/bin/env bash
# Runs an example APK of package `rs.pushups.example` on the device adb selects with real FCM
# pushes: one to the open app, one that starts the app after its process is killed, and the drain
# when the app opens. The app logs under the logcat tag `pushups-example`.
#
# Usage: ci/android/example.sh <apk> <FCM service account JSON> [launch Activity]
# The Activity defaults to the one `dx` generates for examples/dioxus.
set -euo pipefail

apk=$1
key=$(realpath "$2")
ci=$(cd "$(dirname "$0")" && pwd)
package=rs.pushups.example
activity=$package/${3:-dev.dioxus.main.MainActivity}
STEP_S=180
# FCM refuses a fresh emulator's first registrations for some seconds after boot, so failed attempts are spaced out over this window.
REGISTER_S=240

log() { adb logcat -d -s pushups-example:I; }
fail() {
  echo "example: $1" >&2
  log | tail -40 >&2
  exit 1
}
# Prints the first pushups-example line matching the extended regex $1, waiting up to STEP_S for it.
await() {
  local deadline=$((SECONDS + STEP_S))
  while [ $SECONDS -lt $deadline ]; do
    if log | grep -qE "$1"; then
      log | grep -E "$1" | head -1
      return 0
    fi
    sleep 1
  done
  fail "no line matching '$1' within $STEP_S s"
}
open_app() {
  adb shell input keyevent KEYCODE_WAKEUP
  adb shell wm dismiss-keyguard
  adb shell am start -W -n "$activity" >/dev/null
}
# Prints the FCM token, reopening the app after a pause each time its registration fails, until REGISTER_S passes.
register() {
  local attempt=1 pause=10 line deadline=$((SECONDS + REGISTER_S))
  while :; do
    adb logcat -c
    open_app >&2
    line=$(await 'ui token at_ms=[0-9]+ Fcm\(|ui registration failed')
    case "$line" in
      *"ui token"*)
        echo "$line" | sed -E 's/.*Fcm\("([^"]+)"\).*/\1/'
        return 0
        ;;
    esac
    [ $SECONDS -lt $deadline ] || fail "FCM registration failed $attempt times in $REGISTER_S s"
    echo "example: registration attempt $attempt failed, reopening the app in $pause s" >&2
    adb shell am force-stop "$package"
    sleep "$pause"
    attempt=$((attempt + 1))
    pause=$((pause < 30 ? pause + 10 : 30))
  done
}

adb uninstall "$package" >/dev/null 2>&1 || true
adb install "$apk" >/dev/null
# Before Android 13 there is no permission to grant.
adb shell pm grant "$package" android.permission.POST_NOTIFICATIONS 2>/dev/null || true
token=$(register)
[ -n "$token" ] || fail "no FCM token"

python3 "$ci/fcm_push.py" "$key" "$token" 1
await 'ui message .*started_app=false seq=1 '
await 'handler started_app=false .*seq=1 '

adb shell input keyevent KEYCODE_HOME
# Killed as the system kills a background app, which FCM still delivers to, unlike a force stop.
# `am kill` spares a process whose oom adj is still below SERVICE_ADJ, as during the home transition, so it is repeated until the process is gone.
deadline=$((SECONDS + 30))
until [ -z "$(adb shell pidof "$package" || true)" ]; do
  [ $SECONDS -lt $deadline ] || fail "the process outlived am kill for 30 s"
  adb shell am kill "$package"
  sleep 1
done
python3 "$ci/fcm_push.py" "$key" "$token" 2
await 'handler started_app=true .*seq=2 '

open_app
await 'ui message .*started_app=true seq=2 '
echo "example: every step passed"
