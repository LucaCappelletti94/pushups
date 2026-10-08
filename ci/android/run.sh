#!/usr/bin/env bash
# Runs the Android instrumented tests on the device adb selects (set ANDROID_SERIAL when several are
# attached) with real FCM and UnifiedPush pushes, and writes target/<name>-rust.lcov and
# target/<name>-kotlin.xml.
#
# Usage: ci/android/run.sh <x86_64|arm64-v8a> <FCM service account JSON> [name, default android]
# Needs ANDROID_HOME, ANDROID_NDK_HOME, a JDK 17 or later, the Rust target for the ABI, rustup's
# llvm-tools, openssl, and the Firebase configuration at ci/android/probe/google-services.json.
set -euo pipefail

abi=$1
key=$(realpath "$2")
name=${3:-android}
ci=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$ci/../.." && pwd)
build=$ci/build
out=$build/runs
package=rs.pushups.example
files=/data/data/$package/files

case $abi in
  x86_64) target=x86_64-linux-android ;;
  arm64-v8a) target=aarch64-linux-android ;;
  *) echo "unknown ABI $abi" >&2; exit 2 ;;
esac
linker=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/${target}24-clang
target_env=$(echo "$target" | tr '[:lower:]-' '[:upper:]_')

rm -rf "$out"
mkdir -p "$out" "$root/target"

# ntfy is the UnifiedPush distributor of the UnifiedPush variant, pinned to an F-Droid build.
ntfy_version=63
ntfy_sha256=b4baa6f668bd57ad5df3b4e0e769165dcf25c5ee19fdb04f78077ee819616f2a
if ! adb shell pm list packages io.heckel.ntfy | grep -q io.heckel.ntfy; then
  apk=$build/ntfy-$ntfy_version.apk
  [ -f "$apk" ] || curl -fsSL -o "$apk" "https://f-droid.org/repo/io.heckel.ntfy_$ntfy_version.apk"
  echo "$ntfy_sha256  $apk" | sha256sum -c --quiet
  adb install "$apk" >/dev/null
fi

# The app server's side of Web Push: a fresh VAPID key pair and the `web-push` crate's sender.
openssl ecparam -name prime256v1 -genkey -noout -out "$out/vapid.pem"
vapid_hex=$(openssl ec -in "$out/vapid.pem" -pubout -outform DER 2>/dev/null | tail -c 65 | od -An -v -tx1 | tr -d ' \n')
(cd "$root/ci/linux/probe" && cargo build --quiet --bin send)

# The screen stays on and unlocked while the tests run, since install checks, the prompt and the
# shade show on it. A variant's build takes minutes, so each one wakes the device again.
wake() {
  adb shell input keyevent KEYCODE_WAKEUP
  adb shell wm dismiss-keyguard
}
adb shell svc power stayon true

# A distributor never opened stays in Android's stopped state, which drops the app's registration
# broadcast, so ntfy is opened once, as its user would, with its own permission prompt pre-answered.
adb shell pm grant io.heckel.ntfy android.permission.POST_NOTIFICATIONS 2>/dev/null || true
# ntfy's first screen asks for this. Without it, Android 12 and later refuse ntfy's subscriber
# service from the background, and ntfy.sh then refuses pushes to a topic with no subscriber.
adb shell dumpsys deviceidle whitelist +io.heckel.ntfy >/dev/null
wake
adb shell am start -W -n io.heckel.ntfy/.ui.MainActivity >/dev/null
adb shell input keyevent KEYCODE_HOME

# Request files a previous run left on the device would be sent ahead of this run's, so they go.
adb uninstall "$package" >/dev/null 2>&1 || true
python3 "$ci/send.py" "$key" --webpush-sender "$root/ci/linux/probe/target/debug/send" --vapid "$out/vapid.pem" &
sender=$!
trap 'kill $sender 2>/dev/null || true; adb shell pm enable io.heckel.ntfy >/dev/null 2>&1 || true; adb shell svc power stayon false || true' EXIT

# One app configuration: the probe built with <features>, the test manifest <manifest>, the test
# class <class>, and instrumentation arguments as key and value pairs. Each runs in a fresh
# install, so the notification permission starts ungranted, and leaves its profiles in $out.
run_variant() {
  local variant=$1 features=$2 manifest=$3 class=$4
  shift 4
  local arguments=()
  while [ "$#" -ge 2 ]; do
    arguments+=(-e "$1" "'$2'")
    shift 2
  done
  echo "== $variant"
  (
    cd "$ci/probe"
    env "CARGO_TARGET_${target_env}_LINKER=$linker" RUSTFLAGS="-Cinstrument-coverage" \
      cargo build --quiet --target "$target" ${features:+--features "$features"}
  )
  rm -rf "$build/jniLibs"
  mkdir -p "$build/jniLibs/$abi"
  cp "$ci/probe/target/$target/debug/libpushups_probe.so" "$build/jniLibs/$abi/"
  cp "$ci/probe/target/$target/debug/libpushups_probe.so" "$out/$variant.so"
  (cd "$ci" && ./gradlew --no-daemon --quiet -Ppushups.manifest="$manifest" assembleDebugAndroidTest)

  wake
  adb uninstall "$package" >/dev/null 2>&1 || true
  adb install -t "$build/pushups/outputs/apk/androidTest/debug/pushups-debug-androidTest.apk" >/dev/null
  timeout 900 adb shell am instrument -w -e class "rs.pushups.ci.$class" "${arguments[@]}" \
    -e coverage true -e coverageFile "$files/jacoco.ec" "$package/androidx.test.runner.AndroidJUnitRunner" \
    | tee "$out/$variant.txt" || true
  adb exec-out run-as "$package" cat files/pushups.profraw > "$out/$variant.profraw"
  adb exec-out run-as "$package" cat files/jacoco.ec > "$out/$variant.ec"
}

run_variant full "config-real background-handler" default PushupsTest
run_variant fallback-name config-real fallback-name FallbackNameTest
run_variant no-background-handler config-real default NoBackgroundHandlerTest
run_variant unconfigured "" default RegistrationFailsTest expect "FCM is not configured"
run_variant mismatched config-mismatched default RegistrationFailsTest expect "FCM is not configured"
run_variant rejected config-fixture default RegistrationFailsTest expect "Please set a valid API key"
run_variant unifiedpush "config-real background-handler" default UnifiedPushTest vapid "$vapid_hex" endpoint https://ntfy.sh/
# With ntfy disabled the device has no distributor, so the same configuration registers through FCM.
adb shell pm disable-user --user 0 io.heckel.ntfy >/dev/null
run_variant unifiedpush-fallback "config-real background-handler" default UnifiedPushFallbackTest vapid "$vapid_hex"
adb shell pm enable io.heckel.ntfy >/dev/null

# Rust: every variant's profile against its own library, kept to the crate's own sources, with
# paths relative to the repository.
tools=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin
"$tools/llvm-profdata" merge -sparse "$out"/*.profraw -o "$out/pushups.profdata"
libraries=("$out"/*.so)
objects=("${libraries[0]}")
for so in "${libraries[@]:1}"; do
  objects+=(-object "$so")
done
"$tools/llvm-cov" export -format=lcov -instr-profile "$out/pushups.profdata" "${objects[@]}" "$root/src" \
  | sed "s|^SF:$root/|SF:|" > "$root/target/$name-rust.lcov"

# Kotlin: the module's classes against every JaCoCo profile the runner wrote.
jacoco=$(sed -n 's/^pushups\.jacoco=//p' "$ci/gradle.properties")
cli=$build/jacococli-$jacoco.jar
[ -f "$cli" ] || curl -fsSL -o "$cli" \
  "https://repo1.maven.org/maven2/org/jacoco/org.jacoco.cli/$jacoco/org.jacoco.cli-$jacoco-nodeps.jar"
java -jar "$cli" report "$out"/*.ec \
  --classfiles "$build/pushups/tmp/kotlin-classes/debug" \
  --sourcefiles "$root/android/src/main/kotlin" \
  --xml "$root/target/$name-kotlin.xml" >/dev/null

failed=0
for log in "$out"/*.txt; do
  grep -q '^OK (' "$log" || { echo "failed: $(basename "$log" .txt)" >&2; failed=1; }
done
exit $failed
