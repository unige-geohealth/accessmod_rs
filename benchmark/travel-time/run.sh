#!/bin/sh
# Travel-time benchmark, AccessMod 5 vs accessmod-rs; writes
# benchmark/travel-time/report.md (see README.md there).
#
#   sh benchmark/travel-time/run.sh [side ...]     (default: 1000 2000 4000)
#
# REPEAT (default 3) runs per measurement; AM5_MEMORY_MB (default 8000) is
# the memory given to r.walk.accessmod, enough for it never to spill.
set -eu
. "$(dirname "$0")/../lib.sh"
AM5_MEMORY_MB=${AM5_MEMORY_MB:-8000}
[ "$#" -gt 0 ] || set -- 1000 2000 4000
build
work=$(scratch travel-time)
report="$bench/travel-time/report.md"
{
    header "Travel time: AccessMod 5 vs accessmod-rs" "sh benchmark/travel-time/run.sh $*"
    echo "## Demo project, full analysis"
    echo
    echo "| Analysis | AM5 replay | AM5 \`global.R\` load (once) | accessmod-rs | rs peak RSS (MB) |"
    echo "| --- | ---: | ---: | ---: | ---: |"
} > "$report"

mkdir -p "$work/demo/am5"
am5_replay "$work/demo/am5" benchmark/travel-time/demo-replay.json
read -r rs_s rs_mb <<EOF2
$(repeat_rs "$work/demo" rs "$work/demo/out" "$bin/accessmod" travel-time run --json \
    --job "$bench/travel-time/demo-job.json")
EOF2
printf '| 227 × 196 cells, 10 facilities, 120 min | %s | %s | %s | %s |\n\n' \
    "$(median < "$work/demo/am5/seconds.txt")" "$(cat "$work/demo/am5/load.txt")" "$rs_s" "$rs_mb" \
    >> "$report"

{
    echo "## Synthetic landscapes, engine"
    echo
    echo "| Cells | AM5 \`r.walk.accessmod\` | AM5 peak RSS (MB) | accessmod-rs total | rs solve | rs peak RSS (MB) | max rel. diff |"
    echo "| ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
} >> "$report"
for side in "$@"; do
    dir="$work/$side"
    mkdir -p "$dir/in" "$dir/am5"
    "$bin/examples/synthetic_inputs" "$dir/in" "$side" "$side" 10 2>/dev/null
    AM5_MEMORY_MB=$AM5_MEMORY_MB am5 "$dir/in" "$dir/am5" travel-time/am5.sh
    am5_mb=$(awk -F': ' '/Maximum resident set size/ { printf "%.0f", $2 / 1024 }' "$dir/am5/time.txt")
    read -r rs_s rs_mb <<EOF2
$(repeat_rs "$dir" rs "$dir/in/out" "$bin/accessmod" travel-time run --json --job "$dir/in/job.json")
EOF2
    solve=$(round2 "$(json_number solve "$dir/rs.json")")
    diff=$("$bin/examples/compare_rasters" "$dir/am5/travel_time.tif" "$dir/in/out/travel_time_seconds.tif")
    printf '| %s | %s | %s | %s | %s | %s | %s |\n' "$(cells $((side * side)))" \
        "$(median < "$dir/am5/seconds.txt")" "$am5_mb" "$rs_s" "$solve" "$rs_mb" "$diff" >> "$report"
done
echo "wrote $report"
