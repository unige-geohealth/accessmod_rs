#!/bin/sh
# Runs inside the pinned AccessMod 5 image, in a GRASS session on a fresh
# location created from /in/dem.tif (see am5-vs-rs.sh). Imports the inputs,
# builds the speed map as amCreateSpeedMap does, then times
# r.walk.accessmod alone (the engine) and exports its raw travel time.
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
/usr/bin/time -v r.walk.accessmod -s -k -t --quiet elevation=dem friction=speed \
    output=tt nearest=nearest start_points=hf memory="$AM5_MEMORY_MB" max_cost=0 \
    2> /out/am5-time.txt
end=$(now)
python3 -c "print(f'{$end - $start:.3f}')" > /out/am5-seconds.txt
r.out.gdal -c -m --quiet input=tt type=Float64 output=/out/am5-travel_time_seconds.tif \
    format=GTiff createopt=COMPRESS=DEFLATE
