#!/usr/bin/env bash
# Builds the Android module as io.github.lucacappelletti94:pushups-android at the crate's version
# into ci/maven/build/repo, and with `dry-run` or `release` uploads it to Maven Central.
#
# Usage: ci/maven/publish.sh local     the repository only, unsigned, for apps to build against
#        ci/maven/publish.sh dry-run   signed, uploaded for validation, then dropped unpublished
#        ci/maven/publish.sh release   signed, uploaded and published once Central validates it
#
# dry-run and release need MAVEN_SIGNING_KEY, MAVEN_SIGNING_KEY_PASSWORD, MAVEN_CENTRAL_USERNAME
# and MAVEN_CENTRAL_PASSWORD (the Central Portal user token).
set -euo pipefail
mode=${1:?local, dry-run or release}
here=$(cd "$(dirname "$0")" && pwd)
api=https://central.sonatype.com/api/v1/publisher
# Validation takes minutes, publishing up to half an hour.
STATUS_BOUND_S=2700

case $mode in
  local) unset MAVEN_SIGNING_KEY ;;
  dry-run | release) : "${MAVEN_SIGNING_KEY:?}" "${MAVEN_SIGNING_KEY_PASSWORD:?}" "${MAVEN_CENTRAL_USERNAME:?}" "${MAVEN_CENTRAL_PASSWORD:?}" ;;
  *) echo "unknown mode $mode" >&2; exit 2 ;;
esac

"$here/../check-versions.sh"
rm -rf "$here/build/repo"
(cd "$here" && ./gradlew --no-daemon --quiet :pushups:publishReleasePublicationToBundleRepository)
[ "$mode" = local ] && exit 0

version=$(cd "$here/../.." && cargo pkgid -p pushups | sed 's/.*[#@]//')
bundle=$here/build/pushups-android-$version.zip
# Central's bundle is the Maven layout with signatures and checksums, without repository metadata.
(cd "$here/build/repo" && rm -f "$bundle" && find io -type f ! -name 'maven-metadata.xml*' | sort | zip -q "$bundle" -@)
token=$(printf '%s:%s' "$MAVEN_CENTRAL_USERNAME" "$MAVEN_CENTRAL_PASSWORD" | base64 -w0)
[ -z "${GITHUB_ACTIONS:-}" ] || echo "::add-mask::$token"
type=AUTOMATIC
[ "$mode" = dry-run ] && type=USER_MANAGED
id=$(curl -fsS -m 300 -H "Authorization: Bearer $token" -F "bundle=@$bundle" \
  "$api/upload?publishingType=$type&name=pushups-android-$version")
echo "deployment $id ($type)"

deadline=$((SECONDS + STATUS_BOUND_S))
while [ $SECONDS -lt $deadline ]; do
  status=$(curl -fsS -m 60 -X POST -H "Authorization: Bearer $token" "$api/status?id=$id")
  state=$(printf '%s' "$status" | python3 -c 'import json,sys; print(json.load(sys.stdin)["deploymentState"])')
  echo "state $state"
  case $state in
    FAILED)
      printf '%s\n' "$status" >&2
      exit 1
      ;;
    VALIDATED)
      if [ "$mode" = dry-run ]; then
        curl -fsS -m 60 -X DELETE -H "Authorization: Bearer $token" "$api/deployment/$id"
        echo "validated and dropped, nothing published"
        exit 0
      fi
      ;;
    PUBLISHED)
      echo "published io.github.lucacappelletti94:pushups-android:$version"
      exit 0
      ;;
  esac
  sleep 15
done
echo "deployment $id still not done after $STATUS_BOUND_S s" >&2
exit 1
