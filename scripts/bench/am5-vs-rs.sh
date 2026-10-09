#!/bin/sh
# Travel-time benchmark: AccessMod 5 (r.walk.accessmod in the pinned Docker
# image) vs accessmod-rs, on the same synthetic landscapes.
#
#   sh scripts/bench/am5-vs-rs.sh [side ...]     (default: 1000 2000 4000)
#
# Each grid is side x side cells of 100 m with 10 facilities; anisotropic,
# towards facilities, 16 neighbours, no limit (docs/tutorial scenario).
# Results go to _shared/bench/<side>/ and a summary table to stdout.
set -eu

IMAGE="fredmoser/accessmod:5.9.2-beta.6"
PLATFORM="linux/arm64"
AM5_MEMORY_MB=${AM5_MEMORY_MB:-8000}
FACILITIES=10

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo build --quiet --release -p accessmod-cli
cargo build --quiet --release -p accessmod-io --examples
bin="$root/target/release"
[ "$#" -gt 0 ] || set -- 1000 2000 4000

printf '| cells | AccessMod 5 r.walk (s) | AM5 max RSS (MB) | accessmod-rs total (s) | rs solve (s) | rs peak RSS (MB) | max rel. diff |\n'
printf '| ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n'
for side in "$@"; do
    dir="$root/_shared/bench/$side"
    rm -rf "$dir"
    mkdir -p "$dir/in" "$dir/am5"
    "$bin/examples/synthetic_inputs" "$dir/in" "$side" "$side" "$FACILITIES" 2>/dev/null

    docker run --rm --platform "$PLATFORM" -e AM5_MEMORY_MB="$AM5_MEMORY_MB" \
        -v "$root/scripts/bench:/scripts:ro" -v "$dir/in:/in:ro" -v "$dir/am5:/out" \
        --entrypoint sh "$IMAGE" -c \
        'grass -c /in/dem.tif -e /tmp/db/loc >/dev/null 2>&1 && grass /tmp/db/loc/PERMANENT --exec sh /scripts/am5-run.sh' \
        > "$dir/am5/log.txt" 2>&1
    am5_s=$(cat "$dir/am5/am5-seconds.txt")
    am5_rss=$(awk -F': ' '/Maximum resident set size/ { printf "%.0f", $2 / 1024 }' "$dir/am5/am5-time.txt")

        { time -p "$bin/accessmod" travel-time run --json --job "$dir/in/job.json" \
        > "$dir/rs-report.json" 2> "$dir/rs-log.txt"; } 2> "$dir/rs-time.txt"
    rs_total=$(sed -n 's/^real //p' "$dir/rs-time.txt")
    rs_solve=$(sed -n 's/.*"solve": \([0-9.]*\).*/\1/p' "$dir/rs-report.json" | head -1)
    rs_rss=$(sed -n 's/.*"peak_rss_bytes": \([0-9]*\).*/\1/p' "$dir/rs-report.json" | awk '{ printf "%.0f", $1 / 1048576 }')
    diff=$("$bin/examples/compare_rasters" "$dir/am5/am5-travel_time_seconds.tif" "$dir/in/out/travel_time_seconds.tif")
    printf '| %s | %s | %s | %s | %s | %s | %s |\n' "$((side * side))" "$am5_s" "$am5_rss" "$rs_total" "$rs_solve" "$rs_rss" "$diff"
done
