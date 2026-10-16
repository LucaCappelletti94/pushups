#!/bin/bash
# Runs inside the test bed (ci/linux/testbed.sh) and walks the probe through the plan's Linux
# delivery table with real pushes through ntfy.sh. Reads /out/vapid.pem and /out/vapid.b64url, writes
# the probe's logs to /out/run and its coverage profiles to /out/profraw.
set -euo pipefail
# shellcheck source=ci/linux/lib.sh
. "$(dirname "$0")/lib.sh"

bin=/work/ci/linux/probe/target/x86_64-unknown-linux-gnu/debug
probe=$bin/pushups-linux-probe
# shellcheck disable=SC2034
send=$bin/send
app_id=rs.pushups.LinuxProbe

export PROBE_OUT=$run_dir
PROBE_VAPID_B64URL=$(cat /out/vapid.b64url)
export PROBE_VAPID_B64URL
export LLVM_PROFILE_FILE=/out/profraw/%p-%m.profraw
# The host's user, whose uid need not be the container's, must be able to delete what lands here.
umask 0000
mkdir -p "$PROBE_OUT" /out/profraw

# A process the bus starts for a push gets the daemon's environment, not this script's.
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    UpdateActivationEnvironment 'a{ss}' 5 \
    HOME "$HOME" XDG_DATA_HOME "$XDG_DATA_HOME" PROBE_OUT "$PROBE_OUT" \
    PROBE_VAPID_B64URL "$PROBE_VAPID_B64URL" LLVM_PROFILE_FILE "$LLVM_PROFILE_FILE"

# shellcheck disable=SC2034
# shellcheck disable=SC2034
fail_name=scenario
fail_logs=("$PROBE_OUT/events.log" "$PROBE_OUT/handled.log" "$PROBE_OUT/shown.log" "$PROBE_OUT/notifier.log")

probe_running() {
    pgrep -f "$probe" >/dev/null
}

stop_ui() {
    touch "$PROBE_OUT/stop"
    wait "$1" || fail "the UI probe exited with failure"
    rm -f "$PROBE_OUT/stop"
}

# The id the notification server gave the notification whose body is the payload $1.
notification_id() {
    line=$(await_line notifier.log "body=$1" 30)
    [[ $line == *'actions=["default", "View"]'* ]] || fail "the notification had no tap action: $line"
    echo "$line" | sed -E 's/^notify id=([0-9]+) .*/\1/'
}

# Clicks the notification $1, as the server's own `ActionInvoked` reports a click.
tap() {
    busctl --user call org.freedesktop.Notifications /org/freedesktop/Notifications \
        rs.pushups.TestNotifier TriggerAction us "$1" default >/dev/null \
        || fail "the notification server refused to click $1"
}

# The test bed has no desktop portal, so `show` takes the `org.freedesktop.Notifications` fallback.
"$bin/notifier" &
notifier=$!
trap 'kill "$notifier" 2>/dev/null || true' EXIT
await_line notifier.log serving 10 >/dev/null

echo "== a UI process registers and receives at once"
"$probe" &
ui=$!
await_line events.log "token https://" 60
[ -f "$XDG_DATA_HOME/dbus-1/services/$app_id.service" ] || fail "install wrote no activation file"
push '{"id":"bound-1"}'
line=$(await_line events.log '"id":"bound-1"' 60)
[[ $line == "message started_app=false "* ]] || fail "bound push reported as: $line"
await_line handled.log '"id":"bound-1"' 10 >/dev/null
await_line shown.log 'shown Ok(()) {"id":"bound-1"}' 10 >/dev/null

echo "== a tap on the notification reaches the running UI"
tap "$(notification_id '{"id":"bound-1"}')"
line=$(await_line events.log 'tap:{"id":"bound-1"}' 30)
[[ $line == "message started_app=false "* ]] || fail "the tap reported as: $line"
stop_ui "$ui"

echo "== a push to a closed app starts it headless, which persists it and exits"
push '{"id":"headless-1"}'
line=$(await_line handled.log '"id":"headless-1"' 90)
[[ $line == "message started_app=true "* ]] || fail "headless push reported as: $line"
grep -qF '"id":"headless-1"' "$PROBE_OUT/events.log" && fail "the headless process emitted to a handler"
deadline=$((SECONDS + 30))
while probe_running && [ "$SECONDS" -lt "$deadline" ]; do sleep 0.5; done
probe_running && fail "the headless process did not exit when idle"

echo "== the next UI process drains the queue"
"$probe" &
ui=$!
line=$(await_line events.log '"id":"headless-1"' 30)
[[ $line == "message started_app=true "* ]] || fail "drained push reported as: $line"
stop_ui "$ui"

echo "== a tap on a notification of a headless process starts the UI with the tap"
push '{"id":"headless-2"}'
await_line shown.log 'shown Ok(()) {"id":"headless-2"}' 90 >/dev/null
tap "$(notification_id '{"id":"headless-2"}')"
line=$(await_line events.log 'tap:{"id":"headless-2"}' 60)
[[ $line == "message started_app=true "* ]] || fail "the tap the started UI drained reported as: $line"
line=$(await_line events.log 'message started_app=true {"id":"headless-2"}' 10)
touch "$PROBE_OUT/stop"
deadline=$((SECONDS + 30))
while probe_running && [ "$SECONDS" -lt "$deadline" ]; do sleep 0.5; done
probe_running && fail "the UI the tap started did not stop"
rm -f "$PROBE_OUT/stop"

echo "== every step passed"
