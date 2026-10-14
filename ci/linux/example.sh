#!/bin/bash
# Walks one example app, winit or tauri, through the plan's Linux delivery table with real pushes
# through ntfy.sh, inside the test bed (ci/linux/testbed.sh). The app is built on the host with
# PUSHUPS_VAPID_PUBLIC_KEY and runs from /work. Reads /out/vapid.pem and writes the app's log to
# /out/run.
set -euo pipefail
# shellcheck source=ci/linux/lib.sh
. "$(dirname "$0")/lib.sh"

example=${1:?usage: example.sh winit|tauri}
case "$example" in
winit) bin=/work/examples/winit/target/release/pushups-winit-example ;;
tauri) bin=/work/examples/tauri/target/release/pushups-tauri-example ;;
*) echo "example.sh: expected winit or tauri, got $example" >&2; exit 2 ;;
esac

out=$run_dir
# shellcheck disable=SC2034
send=/work/ci/linux/probe/target/release/send
app_id=rs.pushups.example

umask 0000
mkdir -p "$out"
rm -f "$out/pushups-example.log" "$out/subscription.json"

# shellcheck disable=SC2034
# shellcheck disable=SC2034
fail_name=example
fail_logs=("$out/pushups-example.log")

# A virtual display for the window, since the container has no one to show it in.
Xvfb :99 -screen 0 1024x768x24 &
xvfb=$!
trap 'kill "$xvfb" 2>/dev/null || true' EXIT
deadline=$((SECONDS + 30))
while [ ! -e /tmp/.X11-unix/X99 ] && [ "$SECONDS" -lt "$deadline" ]; do sleep 0.2; done
[ -e /tmp/.X11-unix/X99 ] || fail "Xvfb did not start within 30 s"

export TMPDIR="$out"
export DISPLAY=:99
unset WAYLAND_DISPLAY
# Xvfb has no GPU, so WebKit renders with software GL and no compositor.
export LIBGL_ALWAYS_SOFTWARE=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1

# A fresh queue, so an earlier run of the same app id leaves nothing behind.
rm -rf "${XDG_DATA_HOME:?}/$app_id"

# A process the bus starts for a push gets the bus's activation environment, which this sets.
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    UpdateActivationEnvironment 'a{ss}' 6 \
    HOME "$HOME" XDG_DATA_HOME "$XDG_DATA_HOME" TMPDIR "$TMPDIR" \
    DISPLAY "$DISPLAY" LIBGL_ALWAYS_SOFTWARE "$LIBGL_ALWAYS_SOFTWARE" \
    WEBKIT_DISABLE_COMPOSITING_MODE "$WEBKIT_DISABLE_COMPOSITING_MODE"

now_ms() { date +%s%3N; }

app_running() {
    pgrep -f "$bin" >/dev/null
}

stop_ui() {
    kill "$1" 2>/dev/null || true
    deadline=$((SECONDS + 30))
    while app_running && [ "$SECONDS" -lt "$deadline" ]; do sleep 0.5; done
    if app_running; then fail "the UI app did not exit within 30 s"; fi
}

echo "== a UI process registers and receives at once"
"$bin" &
ui=$!
line=$(await_line pushups-example.log "ui token at_ms=" 90 "the app log")
[[ $line == *"subscription={"* ]] || fail "the token line carried no subscription: $line"
[ -f "$XDG_DATA_HOME/dbus-1/services/$app_id.service" ] || fail "install wrote no activation file"
printf '%s\n' "${line#*subscription=}" > "$out/subscription.json"

echo "== a push to the running app reaches the UI and the background handler"
push "{\"seq\":\"1\",\"sent_at_ms\":\"$(now_ms)\"}"
line=$(await_line pushups-example.log "started_app=false seq=1 " 60 "the app log")
[[ $line == *"ui message"* ]] || fail "the seq=1 UI line is not a message: $line"
line=$(await_line pushups-example.log "handler started_app=false" 30 "the app log")
[[ $line == *"seq=1 "* ]] || fail "the background handler line is not seq=1: $line"

echo "== a push to a closed app starts it headless, which persists it and exits"
stop_ui "$ui"
push "{\"seq\":\"2\",\"sent_at_ms\":\"$(now_ms)\"}"
line=$(await_line pushups-example.log "handler started_app=true" 90 "the app log")
[[ $line == *"seq=2 "* ]] || fail "the headless handler line is not seq=2: $line"
# Only the next UI session may see seq=2 as a message.
if grep -qF "started_app=true seq=2 " "$out/pushups-example.log"; then
    fail "the headless process emitted seq=2 to a UI handler"
fi
deadline=$((SECONDS + 60))
while app_running && [ "$SECONDS" -lt "$deadline" ]; do sleep 0.5; done
if app_running; then fail "the headless process did not exit when idle"; fi

echo "== the next UI process drains the queue"
"$bin" &
ui=$!
line=$(await_line pushups-example.log "started_app=true seq=2 " 60 "the app log")
[[ $line == *"ui message"* ]] || fail "the drained line is not a UI message: $line"
stop_ui "$ui"

echo "== every step passed"
