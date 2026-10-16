#!/bin/sh
# Optimization benchmark on the AccessMod 5 demo, AccessMod 5 vs
# accessmod-rs; writes benchmark/optimization/report.md (see README.md
# there).
#
#   sh benchmark/optimization/run.sh [spacing_m radius_m facilities]
#
# The second case uses discs of radius_m (default 10000) on a lattice of
# spacing_m (default 8000) as catchments, selecting `facilities` (default
# 100). REPEAT (default 3) runs per measurement.
set -eu
. "$(dirname "$0")/../lib.sh"
spacing=${1:-8000}
radius=${2:-10000}
facilities=${3:-100}
build
work=$(scratch optimization)
report="$bench/optimization/report.md"
{
    header "Optimization: AccessMod 5 vs accessmod-rs" "sh benchmark/optimization/run.sh $spacing $radius $facilities"
    echo "## Demo project, full analysis"
    echo
    echo "| Analysis | AM5 | AM5 \`global.R\` load | accessmod-rs | rs peak RSS (MB) |"
    echo "| --- | ---: | ---: | ---: | ---: |"
} > "$report"

mkdir -p "$work/demo/am5"
am5_replay "$work/demo/am5" scripts/oracle/optimization-demo.json
read -r rs_s rs_mb <<EOF2
$(repeat_rs "$work/demo" rs "$work/demo/out" "$bin/accessmod" optimization run --json \
    --job "$bench/optimization/demo-job.json")
EOF2
printf '| 227 × 196 cells, 10 catchments, 10 facilities, 1 per zone (4 zones) | %s | %s | %s | %s |\n' \
    "$(median < "$work/demo/am5/seconds.txt")" "$(cat "$work/demo/am5/load.txt")" "$rs_s" "$rs_mb" \
    >> "$report"

mkdir -p "$work/discs/am5"
am5_session "$work/discs/am5" benchmark/optimization/discs.R "$spacing" "$radius" "$facilities"
discs=$(ogrinfo -so -al "$work/discs/am5/run/discs.fgb" | sed -n 's/^Feature Count: //p')
cat > "$work/discs/job.json" <<EOF2
{
  "schema_version": 1,
  "population": "$root/_oracle/population-correction/demo/input/population.tif",
  "catchments": { "path": "$work/discs/am5/run/discs.fgb", "id_field": "cat_join" },
  "max_facilities": $facilities,
  "output_dir": "$work/discs/out"
}
EOF2
read -r rs_s rs_mb <<EOF2
$(repeat_rs "$work/discs" rs "$work/discs/out" "$bin/accessmod" optimization run --json \
    --job "$work/discs/job.json")
EOF2
printf '| %s discs of %s km on a %s km lattice, %s facilities | %s | %s | %s | %s |\n' \
    "$discs" "$((radius / 1000))" "$((spacing / 1000))" "$facilities" \
    "$(median < "$work/discs/am5/seconds.txt")" "$(cat "$work/discs/am5/load.txt")" "$rs_s" "$rs_mb" \
    >> "$report"
# Same ranking as AccessMod 5 (identifiers in rank order).
cut -d, -f2 "$work/discs/am5/run/optimization.csv" | tail -n +2 | tr -d '"' > "$work/discs/am5.ids"
cut -d, -f2 "$work/discs/out/optimization.csv" | tail -n +2 > "$work/discs/rs.ids"
if cmp -s "$work/discs/am5.ids" "$work/discs/rs.ids"; then same=yes; else same=no; fi
{
    echo
    echo "Discs: same facilities in the same order as AccessMod 5: $same."
} >> "$report"
echo "wrote $report"
