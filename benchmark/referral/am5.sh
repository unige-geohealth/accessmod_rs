#!/bin/sh
# Inside the AccessMod 5 image (see run.sh): times the engine part of an
# AccessMod 5 referral, $REPEAT times: one r.walk.accessmod per origin,
# stopped at the destinations (stop_points), $AM5_JOBS at a time. The
# r.path / v.net distance steps and the R orchestration of a real referral
# are not run: a lower bound of AccessMod 5's cost. Anisotropic, 16
# neighbours, no limit. Writes /out/seconds.txt (one line per run).
set -eu
. /bench/am5/landscape.sh
: > /out/seconds.txt
k=1
while [ "$k" -le "$REPEAT" ]; do
    start=$(now)
    v.category --quiet input=hf option=print | sort -n | xargs -P "$AM5_JOBS" -I{} sh -c '
        v.extract --quiet --overwrite input=hf cats={} output=from_{} &&
        r.walk.accessmod -s -k --quiet --overwrite elevation=dem friction=speed \
            output=tt_{} start_points=from_{} stop_points=hf memory="$AM5_MEMORY_MB" max_cost=0'
    elapsed "$start" >> /out/seconds.txt
    k=$((k + 1))
done
