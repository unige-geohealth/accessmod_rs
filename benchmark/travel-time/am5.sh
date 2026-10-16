#!/bin/sh
# Inside the AccessMod 5 image (see run.sh): times r.walk.accessmod alone,
# the engine of an AccessMod 5 travel time, $REPEAT times, and exports the
# raw travel time (seconds) of the last run. Anisotropic, towards
# facilities, 16 neighbours, no limit. Writes /out/seconds.txt (one line per
# run), /out/time.txt (GNU time of the last run) and /out/travel_time.tif.
set -eu
. /bench/am5/landscape.sh
: > /out/seconds.txt
k=1
while [ "$k" -le "$REPEAT" ]; do
    start=$(now)
    /usr/bin/time -v r.walk.accessmod -s -k -t --quiet --overwrite elevation=dem \
        friction=speed output=tt nearest=nearest start_points=hf \
        memory="$AM5_MEMORY_MB" max_cost=0 2> /out/time.txt
    elapsed "$start" >> /out/seconds.txt
    k=$((k + 1))
done
r.out.gdal -c -m --quiet input=tt type=Float64 output=/out/travel_time.tif \
    format=GTiff createopt=COMPRESS=DEFLATE
