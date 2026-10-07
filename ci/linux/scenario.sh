#!/bin/bash
# Runs inside the test bed (ci/linux/testbed.sh) and walks the probe through the plan's Linux
# delivery table with real pushes through ntfy.sh. Reads /out/vapid.pem and /out/vapid.hex, writes
# the probe's logs to /out/run and its coverage profiles to /out/profraw.
set -euo pipefail

bin=/work/ci/linux/probe/target/x86_64-unknown-linux-gnu/debug
probe=$bin/pushups-linux-probe
send=$bin/send
app_id=rs.pushups.LinuxProbe

export PROBE_OUT=/out/run
export PROBE_VAPID_HEX=$(cat /out/vapid.hex)
export LLVM_PROFILE_FILE=/out/profraw/%p-%m.profraw
mkdir -p "$PROBE_OUT" /out/profraw

# A process the bus starts for a push gets the daemon's environment, not this script's.
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    UpdateActivationEnvironment 'a{ss}' 5 \
    HOME "$HOME" XDG_DATA_HOME "$XDG_DATA_HOME" PROBE_OUT "$PROBE_OUT" \
    PROBE_VAPID_HEX "$PROBE_VAPID_HEX" LLVM_PROFILE_FILE "$LLVM_PROFILE_FILE"

fail() {
    echo "scenario: $*" >&2
    for log in events handled; do
        echo "--- $log.log" >&2
        cat "$PROBE_OUT/$log.log" >&2 2>/dev/null || true
    done
    echo "--- distributor" >&2
    cat /tmp/pw-testbed-distributor.log >&2 || true
    exit 1
}

# Waits up to $3 seconds for a line of file $1 containing $2.
await_line() {
    local deadline=$((SECONDS + $3))
    while [ "$SECONDS" -lt "$deadline" ]; do
        if grep -qF -- "$2" "$PROBE_OUT/$1" 2>/dev/null; then
            grep -F -- "$2" "$PROBE_OUT/$1" | tail -1
            return 0
        fi
        sleep 0.2
    done
    fail "no line with '$2' in $1 within $3 s"
}

push() {
    "$send" "$PROBE_OUT/subscription.json" /out/vapid.pem "$1" || fail "the push service refused $1"
}

probe_running() {
    pgrep -f "$probe" >/dev/null
}

stop_ui() {
    touch "$PROBE_OUT/stop"
    wait "$1" || fail "the UI probe exited with failure"
    rm -f "$PROBE_OUT/stop"
}

echo "== a UI process registers and receives at once"
"$probe" &
ui=$!
await_line events.log "token https://" 60
[ -f "$XDG_DATA_HOME/dbus-1/services/$app_id.service" ] || fail "install wrote no activation file"
push '{"id":"bound-1"}'
line=$(await_line events.log '"id":"bound-1"' 60)
[[ $line == "message started_app=false "* ]] || fail "bound push reported as: $line"
await_line handled.log '"id":"bound-1"' 10 >/dev/null
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

echo "== every step passed"
