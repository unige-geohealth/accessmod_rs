#!/bin/sh
# Geographic coverage benchmark, AccessMod 5 vs accessmod-rs; writes
# benchmark/geographic-coverage/report.md (see README.md there).
#
#   sh benchmark/geographic-coverage/run.sh [side facilities ...]   (default: 1000 20 2000 50 4000 100)
#
# REPEAT (default 3) runs per measurement; AM5_MEMORY_MB (default 2000) is
# the memory of each r.walk.accessmod.
set -eu
. "$(dirname "$0")/../lib.sh"
[ "$#" -ge 2 ] || set -- 1000 20 2000 50 4000 100
build
work=$(scratch geographic-coverage)
report="$bench/geographic-coverage/report.md"
{
    header "Geographic coverage: AccessMod 5 vs accessmod-rs" \
        "sh benchmark/geographic-coverage/run.sh $*"
    echo "## Demo project, full analysis"
    echo
    echo "| Analysis | AM5 replay | AM5 \`global.R\` load (once) | accessmod-rs | rs peak RSS (MB) |"
    echo "| --- | ---: | ---: | ---: | ---: |"
} > "$report"

mkdir -p "$work/demo/am5"
am5_replay "$work/demo/am5" scripts/oracle/capacity-demo.json
read -r rs_s rs_mb <<EOF2
$(repeat_rs "$work/demo" rs "$work/demo/out" "$bin/accessmod" geographic-coverage run --json \
    --job "$bench/geographic-coverage/demo-job.json")
EOF2
printf '| 227 × 196 cells, 10 facilities by capacity, 120 min, catchments | %s | %s | %s | %s |\n\n' \
    "$(median < "$work/demo/am5/seconds.txt")" "$(cat "$work/demo/am5/load.txt")" "$rs_s" "$rs_mb" \
    >> "$report"

{
    echo "## Synthetic landscapes, engine"
    echo
    echo "| Cells | Facilities | AM5 \`r.walk.accessmod\` × N, sequential (lower bound) | accessmod-rs total | rs coverage | rs peak RSS (MB) | Cells reached |"
    echo "| ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
} >> "$report"
while [ "$#" -ge 2 ]; do
    side=$1 count=$2
    shift 2
    dir="$work/$side-$count"
    mkdir -p "$dir/in" "$dir/am5"
    "$bin/examples/synthetic_inputs" "$dir/in" "$side" "$side" "$count" 2>/dev/null
    cat > "$dir/in/coverage.json" <<JOB
{
  "schema_version": 1,
  "dem": "dem.tif",
  "land_cover": "land_cover.tif",
  "population": "population.tif",
  "facilities": { "path": "facilities.gpkg", "id_field": "code" },
  "capacity_field": "capacity",
  "name_field": "name",
  "order": { "field": { "field": "capacity", "descending": true } },
  "scenario": [
    { "class": 1, "mode": "walking", "speed_kmh": 5 },
    { "class": 2, "mode": "walking", "speed_kmh": 4 },
    { "class": 3, "mode": "walking", "speed_kmh": 2 },
    { "class": 1001, "mode": "motorized", "speed_kmh": 60 },
    { "class": 1002, "mode": "bicycling", "speed_kmh": 15 }
  ],
  "method": "anisotropic",
  "direction": "towards_facilities",
  "neighbourhood": 16,
  "limit_s": 7200,
  "output_dir": "out"
}
JOB
    am5 "$dir/in" "$dir/am5" geographic-coverage/am5.sh
    read -r rs_s rs_mb <<EOF2
$(repeat_rs "$dir" rs "$dir/in/out" "$bin/accessmod" geographic-coverage run --json \
    --job "$dir/in/coverage.json")
EOF2
    printf '| %s | %s | %s | %s | %s | %s | %s |\n' "$(cells $((side * side)))" "$count" \
        "$(median < "$dir/am5/seconds.txt")" "$rs_s" \
        "$(round2 "$(json_number coverage "$dir/rs.json")")" "$rs_mb" \
        "$(json_number reached_cells "$dir/rs.json")" >> "$report"
done
echo "wrote $report"
