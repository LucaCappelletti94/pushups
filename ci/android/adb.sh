#!/usr/bin/env bash
# GitHub's hosted runners drop the emulator's adb connection for under a second right after an
# instrumentation ends, while the device and adbd keep running.
set -euo pipefail

# Runs <command>, writing its output to <file>, and runs it once more after adb reaches the device
# again when the first run found the device offline or gone.
reconnecting() {
  local file=$1
  shift
  local status=0
  "$@" >"$file" 2>"$file.err" || status=$?
  if [ "$status" -ne 0 ] && grep -qE "device offline|device '[^']*' not found" "$file.err"; then
    cat "$file.err" >&2
    timeout 60 adb wait-for-device
    status=0
    "$@" >"$file" 2>"$file.err" || status=$?
  fi
  cat "$file.err" >&2
  rm "$file.err"
  return "$status"
}
