#!/bin/sh
# Referral benchmark: all facilities to all facilities on synthetic
# landscapes, AccessMod 5 engine lower bound (am5-referral-run.sh) vs
# accessmod-rs `referral run`.
#
#   sh scripts/bench/referral-vs-am5.sh side facilities [side facilities ...]
#
# Anisotropic, 16 neighbours, tutorial scenario. Results in
# _shared/bench/referral-<side>-<facilities>/, summary table on stdout.
set -eu

IMAGE="fredmoser/accessmod:5.9.2-beta.6"
PLATFORM="linux/arm64"
AM5_JOBS=${AM5_JOBS:-9}
AM5_MEMORY_MB=${AM5_MEMORY_MB:-2000}

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo build --quiet --release -p accessmod-cli
cargo build --quiet --release -p accessmod-io --examples
bin="$root/target/release"
[ "$#" -ge 2 ] || set -- 1000 20 2000 50

printf '| cells | facilities | pairs | AM5 r.walk x N, %s parallel (s) | accessmod-rs total (s) | rs solve (s) | rs searches |\n' "$AM5_JOBS"
printf '| ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n'
while [ "$#" -ge 2 ]; do
    side=$1 count=$2
    shift 2
    dir="$root/_shared/bench/referral-$side-$count"
    rm -rf "$dir"
    mkdir -p "$dir/in" "$dir/am5"
    "$bin/examples/synthetic_inputs" "$dir/in" "$side" "$side" "$count" 2>/dev/null
    cat > "$dir/in/referral.json" <<JOB
{
  "schema_version": 1,
  "dem": "dem.tif",
  "land_cover": "land_cover.tif",
  "origins": { "path": "facilities.gpkg", "id_field": "code" },
  "destinations": { "path": "facilities.gpkg", "id_field": "code" },
  "scenario": [
    { "class": 1, "mode": "walking", "speed_kmh": 5 },
    { "class": 2, "mode": "walking", "speed_kmh": 4 },
    { "class": 3, "mode": "walking", "speed_kmh": 2 },
    { "class": 1001, "mode": "motorized", "speed_kmh": 60 },
    { "class": 1002, "mode": "bicycling", "speed_kmh": 15 }
  ],
  "method": "anisotropic",
  "neighbourhood": 16,
  "output_dir": "out"
}
JOB
    docker run --rm --platform "$PLATFORM" -e AM5_JOBS="$AM5_JOBS" -e AM5_MEMORY_MB="$AM5_MEMORY_MB" \
        -v "$root/scripts/bench:/scripts:ro" -v "$dir/in:/in:ro" -v "$dir/am5:/out" \
        --entrypoint sh "$IMAGE" -c \
        'grass -c /in/dem.tif -e /tmp/db/loc >/dev/null 2>&1 && grass /tmp/db/loc/PERMANENT --exec sh /scripts/am5-referral-run.sh' \
        > "$dir/am5/log.txt" 2>&1
    am5_s=$(cat "$dir/am5/am5-seconds.txt")

    { time -p "$bin/accessmod" referral run --json --job "$dir/in/referral.json" \
        > "$dir/rs-report.json" 2> "$dir/rs-log.txt"; } 2> "$dir/rs-time.txt"
    rs_total=$(sed -n 's/^real //p' "$dir/rs-time.txt")
    rs_solve=$(sed -n 's/.*"solve": \([0-9.]*\).*/\1/p' "$dir/rs-report.json" | head -1)
    searches=$(sed -n 's/.*"searches": \([0-9]*\).*/\1/p' "$dir/rs-report.json" | head -1)
    pairs=$(sed -n 's/.*"pairs": \([0-9]*\).*/\1/p' "$dir/rs-report.json" | head -1)
    printf '| %s | %s | %s | %s | %s | %s | %s |\n' "$((side * side))" "$count" "$pairs" "$am5_s" "$rs_total" "$rs_solve" "$searches"
done
