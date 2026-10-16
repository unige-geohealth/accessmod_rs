#!/bin/sh
# Inside the AccessMod 5 image (see run.sh): times the engine part of an
# AccessMod 5 geographic coverage, $REPEAT times: one r.walk.accessmod per
# facility limited to 120 min, one after the other (each facility starts
# from the residual population left by the previous ones). The R
# orchestration, minute rounding, band sums, ring removal and catchment
# polygons are not run: a lower bound of AccessMod 5's cost. Anisotropic,
# towards facilities, 16 neighbours. Writes /out/seconds.txt.
set -eu
. /bench/am5/landscape.sh
: > /out/seconds.txt
k=1
while [ "$k" -le "$REPEAT" ]; do
    start=$(now)
    for cat in $(v.category --quiet input=hf option=print | sort -n); do
        v.extract --quiet --overwrite input=hf cats="$cat" output=from_"$cat"
        r.walk.accessmod -s -k -t --quiet --overwrite elevation=dem friction=speed output=tt \
            start_points=from_"$cat" memory="$AM5_MEMORY_MB" max_cost=7200
    done
    elapsed "$start" >> /out/seconds.txt
    k=$((k + 1))
done
