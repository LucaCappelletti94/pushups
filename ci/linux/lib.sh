#!/bin/bash
# Shared by scenario.sh and example.sh, which run in the same testbed container from the read-only
# /work mount, so one copy of the failure report, the poll, and the push send works for both.
set -euo pipefail

# The run's files live on the writable /out mount.
run_dir=/out/run

# The sourcing script overrides these before any of them runs.
send=
fail_name=
fail_logs=()
fail() {
    echo "$fail_name: $*" >&2
    for log in "${fail_logs[@]}"; do
        echo "--- $(basename "$log")" >&2
        cat "$log" >&2 2>/dev/null || true
    done
    echo "--- distributor" >&2
    cat /tmp/pw-testbed-distributor.log >&2 || true
    exit 1
}

await_line() {
    local deadline=$((SECONDS + $3))
    while [ "$SECONDS" -lt "$deadline" ]; do
        if grep -qF -- "$2" "$run_dir/$1" 2>/dev/null; then
            grep -F -- "$2" "$run_dir/$1" | tail -1
            return 0
        fi
        sleep 0.2
    done
    fail "no line with '$2' in ${4:-$1} within $3 s"
}

push() {
    "$send" "$run_dir/subscription.json" /out/vapid.pem "$1" || fail "the push service refused $1"
}
