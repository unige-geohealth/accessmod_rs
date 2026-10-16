#!/bin/sh
# Scaling-up benchmark on the AccessMod 5 demo, AccessMod 5 replay vs
# accessmod-rs; writes benchmark/scaling-up/report.md (see README.md there).
#
#   sh benchmark/scaling-up/run.sh
#
# REPEAT (default 3) runs per measurement.
set -eu
. "$(dirname "$0")/../lib.sh"
build
work=$(scratch scaling-up)
report="$bench/scaling-up/report.md"
{
    header "Scaling up: AccessMod 5 vs accessmod-rs" "sh benchmark/scaling-up/run.sh"
    echo "## Demo project, full analysis"
    echo
    echo "| Analysis | AM5 replay | AM5 \`global.R\` load | accessmod-rs | rs peak RSS (MB) |"
    echo "| --- | ---: | ---: | ---: | ---: |"
} > "$report"

mkdir -p "$work/demo/am5"
am5_replay "$work/demo/am5" scripts/oracle/scaling-up-demo.json
read -r rs_s rs_mb <<EOF2
$(repeat_rs "$work/demo" rs "$work/demo/out" "$bin/accessmod" scaling-up run --json \
    --job "$bench/scaling-up/demo-job.json")
EOF2
printf '| 227 × 196 cells, 10 existing facilities, 10 added, 3 factors, 120 min | %s | %s | %s | %s |\n' \
    "$(median < "$work/demo/am5/seconds.txt")" "$(cat "$work/demo/am5/load.txt")" "$rs_s" "$rs_mb" \
    >> "$report"
echo "wrote $report"
