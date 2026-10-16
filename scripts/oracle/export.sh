#!/bin/sh
# Export AccessMod 5 oracle artefacts into _oracle/ from the pinned Docker
# image, then verify them against the committed manifest.
#
#   sh scripts/oracle/export.sh          export and verify
#   sh scripts/oracle/export.sh --update export and rewrite the manifest
#
# Requires Docker. The image is pinned by tag and platform and checked by
# image ID: binaries and image IDs differ between platforms.
set -eu

IMAGE="fredmoser/accessmod:5.9.2-beta.6"
PLATFORM="linux/arm64"

root=$(cd "$(dirname "$0")/../.." && pwd)
manifest="$root/scripts/oracle/manifest.sha256"
out="$root/_oracle"

update=0
[ "${1:-}" = "--update" ] && update=1

docker pull --quiet --platform "$PLATFORM" "$IMAGE" >/dev/null
image_id=$(docker image inspect --platform "$PLATFORM" --format '{{.Id}}' "$IMAGE")
expected_id=$(sed -n 's/^# image-id: //p' "$manifest" 2>/dev/null || true)
if [ "$update" -eq 0 ] && [ -n "$expected_id" ] && [ "$image_id" != "$expected_id" ]; then
    echo "error: $IMAGE ($PLATFORM) is $image_id, manifest expects $expected_id" >&2
    exit 1
fi

# Bicycle LUT: compiled into r.walk.accessmod as a float array.
mkdir -p "$out/bicycle-lut"
cid=$(docker create --platform "$PLATFORM" "$IMAGE")
trap 'docker rm -f "$cid" >/dev/null 2>&1 || true' EXIT
docker cp -q "$cid:/usr/local/grass/bin/r.walk.accessmod" "$out/bicycle-lut/r.walk.accessmod"

# Travel times on the demo project (scripts/oracle/travel-time-demo.sh).
rm -rf "$out/travel-time/demo"
docker run --rm --platform "$PLATFORM" \
    -v "$root/scripts/oracle:/scripts:ro" -v "$out:/out" \
    --entrypoint sh "$IMAGE" \
    -c 'grass /data/dbgrass/demo/demo --exec sh /scripts/travel-time-demo.sh' \
    > "$out/travel-time-demo.log" 2>&1 \
    || { echo "error: demo export failed, see $out/travel-time-demo.log" >&2; exit 1; }

# Referral on the demo project (scripts/oracle/referral-demo.R).
rm -rf "$out/referral/demo"
docker run --rm --platform "$PLATFORM" -e TZ=UTC \
    -v "$root/scripts/oracle:/scripts:ro" -v "$out:/out" \
    --entrypoint sh "$IMAGE" \
    -c 'echo UTC > /etc/timezone; cd /app && Rscript /scripts/referral-demo.R' \
    > "$out/referral-demo.log" 2>&1 \
    || { echo "error: referral export failed, see $out/referral-demo.log" >&2; exit 1; }

# Merge land cover on the demo project (scripts/oracle/merge-land-cover-demo.sh).
rm -rf "$out/merge-land-cover/demo"
docker run --rm --platform "$PLATFORM" \
    -v "$root/scripts/oracle:/scripts:ro" -v "$out:/out" \
    --entrypoint sh "$IMAGE" \
    -c 'grass /data/dbgrass/demo/demo --exec sh /scripts/merge-land-cover-demo.sh' \
    > "$out/merge-land-cover-demo.log" 2>&1 \
    || { echo "error: merge export failed, see $out/merge-land-cover-demo.log" >&2; exit 1; }

cd "$out"
if [ "$update" -eq 1 ]; then
    {
        echo "# AccessMod 5 oracle manifest (sh scripts/oracle/export.sh --update)"
        echo "# image: $IMAGE ($PLATFORM)"
        echo "# image-id: $image_id"
        find . -type f ! -name '.*' ! -name '*.log' | LC_ALL=C sort | xargs shasum -a 256
    } > "$manifest"
    echo "wrote $manifest"
else
    grep -v '^#' "$manifest" | shasum -a 256 -c -
fi
