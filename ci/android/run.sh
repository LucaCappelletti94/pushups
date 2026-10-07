#!/usr/bin/env bash
# Runs the Android instrumented tests on the device adb selects (set ANDROID_SERIAL when several are
# attached) with real FCM pushes, and writes target/<name>-rust.lcov and target/<name>-kotlin.xml.
#
# Usage: ci/android/run.sh <x86_64|arm64-v8a> <FCM service account JSON> [name, default android]
# Needs ANDROID_HOME, ANDROID_NDK_HOME, a JDK 17 or later, the Rust target for the ABI, rustup's
# llvm-tools, and the Firebase configuration at ci/android/probe/google-services.json.
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

# The screen stays on and unlocked while the tests run, since install checks, the prompt and the
# shade show on it. A variant's build takes minutes, so each one wakes the device again.
wake() {
  adb shell input keyevent KEYCODE_WAKEUP
  adb shell wm dismiss-keyguard
}
adb shell svc power stayon true

python3 "$ci/send.py" "$key" &
sender=$!
trap 'kill $sender 2>/dev/null || true; adb shell svc power stayon false || true' EXIT

# One app configuration: the probe built with <features>, the test manifest <manifest>, and the test
# class <class>, with <expect> passed to it. Each runs in a fresh install, so the notification
# permission starts ungranted, and leaves its profiles in $out.
run_variant() {
  local variant=$1 features=$2 manifest=$3 class=$4 expect=${5:-}
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
  timeout 900 adb shell am instrument -w -e class "rs.pushups.ci.$class" ${expect:+-e expect "'$expect'"} \
    -e coverage true -e coverageFile "$files/jacoco.ec" "$package/androidx.test.runner.AndroidJUnitRunner" \
    | tee "$out/$variant.txt" || true
  adb exec-out run-as "$package" cat files/pushups.profraw > "$out/$variant.profraw"
  adb exec-out run-as "$package" cat files/jacoco.ec > "$out/$variant.ec"
}

run_variant full "config-real background-handler" default PushupsTest
run_variant fallback-name config-real fallback-name FallbackNameTest
run_variant no-background-handler config-real default NoBackgroundHandlerTest
run_variant unconfigured "" default RegistrationFailsTest "FCM is not configured"
run_variant mismatched config-mismatched default RegistrationFailsTest "FCM is not configured"
run_variant rejected config-fixture default RegistrationFailsTest "Please set a valid API key"

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
