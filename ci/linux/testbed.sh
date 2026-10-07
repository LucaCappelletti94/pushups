#!/bin/sh
# Build the Linux UnifiedPush testbed image if missing and run a command inside it.
# The distributor owns org.unifiedpush.Distributor.kde on a fresh session bus before the command runs.
# The repository is mounted read-only at /work, and PUSHUPS_TESTBED_OUT, when set, writable at /out.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
image=pushups-unifiedpush-testbed

if [ "$#" -lt 1 ]; then
    echo "usage: testbed.sh <command> [args...]" >&2
    exit 2
fi

if ! docker image inspect "$image" >/dev/null 2>&1; then
    docker build -t "$image" "$here"
fi

out_mount=""
if [ -n "${PUSHUPS_TESTBED_OUT:-}" ]; then
    out_mount="-v $PUSHUPS_TESTBED_OUT:/out"
fi

# shellcheck disable=SC2086
exec docker run --rm \
    --user testuser \
    -v "$repo:/work:ro" \
    $out_mount \
    "$image" "$@"
