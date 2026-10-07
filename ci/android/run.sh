#!/usr/bin/env bash
# Runs the Android instrumented tests on the device adb selects (set ANDROID_SERIAL when several are
# attached) with real FCM pushes, and writes target/android-rust.lcov and target/android-kotlin.xml.
#
# Usage: ci/android/run.sh <x86_64|arm64-v8a> <FCM service account JSON>
# Needs ANDROID_HOME, ANDROID_NDK_HOME, a JDK 17 or later, the Rust target for the ABI, rustup's
# llvm-tools, and the Firebase configuration at ci/android/probe/google-services.json.
set -euo pipefail

abi=$1
key=$(realpath "$2")
ci=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$ci/../.." && pwd)
build=$ci/build
package=rs.pushups.example

case $abi in
  x86_64) target=x86_64-linux-android ;;
  arm64-v8a) target=aarch64-linux-android ;;
  *) echo "unknown ABI $abi" >&2; exit 2 ;;
esac

# The probe library, with coverage counters, as the APK's native library.
linker=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/${target}24-clang
target_env=$(echo "$target" | tr '[:lower:]-' '[:upper:]_')
(
  cd "$ci/probe"
  env "CARGO_TARGET_${target_env}_LINKER=$linker" RUSTFLAGS="-Cinstrument-coverage" \
    cargo build --target "$target"
)
so=$ci/probe/target/$target/debug/libpushups_probe.so
rm -rf "$build/jniLibs"
mkdir -p "$build/jniLibs/$abi"
cp "$so" "$build/jniLibs/$abi/"

(cd "$ci" && ./gradlew --no-daemon --quiet assembleDebugAndroidTest)
apk=$build/pushups/outputs/apk/androidTest/debug/pushups-debug-androidTest.apk

# The screen stays on and unlocked, since install checks, the prompt and the shade show on it.
adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard
# A fresh install without -g, so the notification permission starts ungranted and the prompt shows.
adb uninstall "$package" >/dev/null 2>&1 || true
adb install -t "$apk"

python3 "$ci/send.py" "$key" &
sender=$!
trap 'kill $sender 2>/dev/null || true' EXIT

files=/data/data/$package/files
timeout 1500 adb shell am instrument -w -e coverage true -e coverageFile "$files/jacoco.ec" \
  "$package/androidx.test.runner.AndroidJUnitRunner" | tee "$build/instrument.txt"

adb exec-out run-as "$package" cat files/pushups.profraw > "$build/pushups.profraw"
adb exec-out run-as "$package" cat files/jacoco.ec > "$build/jacoco.ec"

# Rust: the probe's profile, kept to the crate's own sources, with paths relative to the repository.
tools=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin
"$tools/llvm-profdata" merge -sparse "$build/pushups.profraw" -o "$build/pushups.profdata"
mkdir -p "$root/target"
"$tools/llvm-cov" export -format=lcov -instr-profile "$build/pushups.profdata" "$so" "$root/src" \
  | sed "s|^SF:$root/|SF:|" > "$root/target/android-rust.lcov"

# Kotlin: the module's classes against the JaCoCo profile the runner wrote.
jacoco=$(sed -n 's/^pushups\.jacoco=//p' "$ci/gradle.properties")
cli=$build/jacococli-$jacoco.jar
[ -f "$cli" ] || curl -fsSL -o "$cli" \
  "https://repo1.maven.org/maven2/org/jacoco/org.jacoco.cli/$jacoco/org.jacoco.cli-$jacoco-nodeps.jar"
java -jar "$cli" report "$build/jacoco.ec" \
  --classfiles "$build/pushups/tmp/kotlin-classes/debug" \
  --sourcefiles "$root/android/src/main/kotlin" \
  --xml "$root/target/android-kotlin.xml"

grep -q '^OK (' "$build/instrument.txt"
