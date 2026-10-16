#!/bin/sh
# Referral benchmark (all facilities to all facilities), AccessMod 5 vs
# accessmod-rs; writes benchmark/referral/report.md (see README.md there).
#
#   sh benchmark/referral/run.sh [side facilities ...]   (default: 1000 20 2000 50 4000 100)
#
# REPEAT (default 3) runs per measurement; AM5_JOBS (default 9) parallel
# r.walk.accessmod, each with AM5_MEMORY_MB (default 2000).
set -eu
. "$(dirname "$0")/../lib.sh"
AM5_JOBS=${AM5_JOBS:-9}
[ "$#" -ge 2 ] || set -- 1000 20 2000 50 4000 100
build
work=$(scratch referral)
report="$bench/referral/report.md"
{
    header "Referral: AccessMod 5 vs accessmod-rs" "sh benchmark/referral/run.sh $*"
    echo "## Demo project, full analysis"
    echo
    echo "| Analysis | AM5 replay | AM5 \`global.R\` load (once) | accessmod-rs | rs peak RSS (MB) |"
    echo "| --- | ---: | ---: | ---: | ---: |"
} > "$report"

mkdir -p "$work/demo/am5"
am5_replay "$work/demo/am5" scripts/oracle/referral-demo.json
read -r rs_s rs_mb <<EOF2
$(repeat_rs "$work/demo" rs "$work/demo/out" "$bin/accessmod" referral run --json \
    --job "$bench/referral/demo-job.json")
EOF2
printf '| 227 × 196 cells, 10 × 10 facilities (90 pairs), 8 neighbours | %s | %s | %s | %s |\n\n' \
    "$(median < "$work/demo/am5/seconds.txt")" "$(cat "$work/demo/am5/load.txt")" "$rs_s" "$rs_mb" \
    >> "$report"

{
    echo "## Synthetic landscapes, engine"
    echo
    echo "| Cells | Facilities | Pairs | AM5 \`r.walk.accessmod\` × N, $AM5_JOBS parallel (lower bound) | accessmod-rs total | rs solve | rs peak RSS (MB) |"
    echo "| ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
} >> "$report"
while [ "$#" -ge 2 ]; do
    side=$1 count=$2
    shift 2
    dir="$work/$side-$count"
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
    AM5_JOBS=$AM5_JOBS am5 "$dir/in" "$dir/am5" referral/am5.sh
    read -r rs_s rs_mb <<EOF2
$(repeat_rs "$dir" rs "$dir/in/out" "$bin/accessmod" referral run --json --job "$dir/in/referral.json")
EOF2
    printf '| %s | %s | %s | %s | %s | %s | %s |\n' "$(cells $((side * side)))" "$count" \
        "$(json_number pairs "$dir/rs.json")" "$(median < "$dir/am5/seconds.txt")" "$rs_s" \
        "$(round2 "$(json_number solve "$dir/rs.json")")" "$rs_mb" >> "$report"
done
echo "wrote $report"
