#!/usr/bin/env bash
# Presentation must be testable without push-provider credentials.
set -euo pipefail

abi=$1
report=${2:-rendering}
ci=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$ci/../.." && pwd)
build=$ci/build-rendering
out=$build/runs
package=rs.pushups.example
files=/data/data/$package/files
bound=900

case $abi in
  x86_64) target=x86_64-linux-android ;;
  arm64-v8a) target=aarch64-linux-android ;;
  *) echo "unknown ABI $abi" >&2; exit 2 ;;
esac
linker=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/${target}24-clang
target_env=$(echo "$target" | tr '[:lower:]-' '[:upper:]_')

rm -rf "$build"
mkdir -p "$out" "$root/target"

(cd "$ci/probe" \
  && env "CARGO_TARGET_${target_env}_LINKER=$linker" RUSTFLAGS="-Cinstrument-coverage" \
     cargo build --quiet --target "$target" --features "config-fixture background-handler")

cp "$ci/probe/target/$target/debug/libpushups_probe.so" "$out/probe.so"
rm -rf "$build/jniLibs"
mkdir -p "$build/jniLibs/$abi"
cp "$ci/probe/target/$target/debug/libpushups_probe.so" "$build/jniLibs/$abi/"

run() {
  local manifest=$1 class=$2 name=$3
  echo "== $name"
  (cd "$ci" && ./gradlew --no-daemon --quiet \
    -Ppushups.buildDir=build-rendering -Ppushups.manifest="$manifest" assembleDebugAndroidTest)
  adb uninstall "$package" >/dev/null 2>&1 || true
  adb install -t "$build/pushups/outputs/apk/androidTest/debug/pushups-debug-androidTest.apk" >/dev/null
  # The shade requires an unlocked screen.
  adb shell input keyevent KEYCODE_WAKEUP
  adb shell wm dismiss-keyguard
  adb logcat -c
  timeout $bound adb shell am instrument -w -e class "rs.pushups.ci.$class" \
    -e coverage true -e coverageFile "$files/jacoco.ec" "$package/androidx.test.runner.AndroidJUnitRunner" \
    | tee "$out/$name.txt" || true
  grep -q '^OK (' "$out/$name.txt" || {
    echo "rendering: $class failed" >&2
    # Native failures can bypass the instrumentation report.
    adb logcat -d -v time -s pushups:V FirebaseApp:V AndroidRuntime:V TestRunner:V | tail -30 >&2
    exit 1
  }
  adb exec-out run-as "$package" cat files/pushups.profraw > "$out/$name.profraw"
  adb exec-out run-as "$package" cat files/jacoco.ec > "$out/$name.ec"
}

# Fresh processes isolate first-launch attribution and disabled-channel settings.
run default PresentationRefusalsTest refusals
run presentation PresentationRefusalsTest refusals-with-launcher
run presentation PresentationTest presentation
run presentation HeadlessPresentationTest headless

tools=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin
"$tools/llvm-profdata" merge -sparse "$out"/*.profraw -o "$out/pushups.profdata"
"$tools/llvm-cov" export -format=lcov -instr-profile "$out/pushups.profdata" \
  "$out/probe.so" "$root/src" \
  | sed "s|^SF:$root/|SF:|" > "$root/target/$report-rust.lcov"

jacoco=$(sed -n 's/^pushups\.jacoco=//p' "$ci/gradle.properties")
cli=$build/jacococli-$jacoco.jar
curl -fsSL -o "$cli" "https://repo1.maven.org/maven2/org/jacoco/org.jacoco.cli/$jacoco/org.jacoco.cli-$jacoco-nodeps.jar"
java -jar "$cli" report "$out"/*.ec \
  --classfiles "$build/pushups/tmp/kotlin-classes/debug" \
  --sourcefiles "$root/android/src/main/kotlin" \
  --xml "$root/target/$report-kotlin.xml" >/dev/null
echo "rendering: every step passed"
