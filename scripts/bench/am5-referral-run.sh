#!/bin/sh
# Runs inside the pinned AccessMod 5 image (see referral-vs-am5.sh). Imports
# the inputs, builds the speed map, then times the engine part of an
# AccessMod 5 referral: one r.walk.accessmod per origin, stopped at the
# destinations (stop_points), $AM5_JOBS at a time. The r.path / v.net
# distance steps and the R orchestration of a real referral are not run:
# this is a lower bound of AccessMod 5's cost.
set -eu

r.in.gdal --quiet -o input=/in/dem.tif output=dem
r.in.gdal --quiet -o input=/in/land_cover.tif output=lc
v.in.ogr --quiet -o input=/in/facilities.gpkg output=hf
g.region raster=dem
: > /tmp/speed.rules
grep -v '^#' /scripts/scenario.txt | while read -r class mode speed; do
    case "$mode" in
        walking) base=1000 ;; bicycling) base=2000 ;; motorized) base=3000 ;;
    esac
    echo "$class = $(( (base + speed) * 1000 ))" >> /tmp/speed.rules
done
r.reclass --quiet input=lc output=speed rules=/tmp/speed.rules

now() { python3 -c 'import time; print(time.time())'; }
start=$(now)
v.category --quiet input=hf option=print | sort -n | xargs -P "$AM5_JOBS" -I{} sh -c '
    v.extract --quiet input=hf cats={} output=from_{} &&
    r.walk.accessmod -s -k --quiet elevation=dem friction=speed output=tt_{} \
        start_points=from_{} stop_points=hf memory="$AM5_MEMORY_MB" max_cost=0'
end=$(now)
python3 -c "print(f'{$end - $start:.3f}')" > /out/am5-seconds.txt
