#!/bin/sh
# Runs inside the pinned AccessMod 5 image, in a GRASS session on the demo
# mapset (see export.sh). Exports the demo inputs and raw AccessMod 5 travel
# times (seconds, before amCleanTravelTime) for every case of
# travel-time-cases.txt, using the scenario of demo-scenario.txt.
#
# Speed and friction maps are built as amCreateSpeedMap and
# amCreateFrictionMap do (tools/R/amFrictionSpeed.R): speed = (mode raster
# value + km/h) * 1000 via r.reclass; friction = seconds per cell via r.recode
# on the CELL land cover.
set -eu

out=/out/travel-time/demo
mkdir -p "$out/input" "$out/oracle"
g.region raster=rDem__dem@PERMANENT

r.out.gdal -c -m --quiet --overwrite input=rDem__dem@PERMANENT \
    output="$out/input/dem.tif" format=GTiff type=Int32 createopt=COMPRESS=DEFLATE
r.out.gdal -c -m --quiet --overwrite input=rLandCoverMerged__demo \
    output="$out/input/land_cover.tif" format=GTiff type=Int32 createopt=COMPRESS=DEFLATE
# FlatGeobuf: carries the CRS WKT (the demo CRS has no EPSG code, which
# GeoJSON cannot express) and, unlike GeoPackage, no timestamps, so the
# manifest stays byte-stable.
rm -f "$out/input/facilities.fgb"
v.out.ogr --quiet input=vFacility__demo output="$out/input/facilities.fgb" format=FlatGeobuf

res=$(g.region -g | sed -n 's/^nsres=//p')
speed_rules=/tmp/speed.rules
friction_rules=/tmp/friction.rules
: > "$speed_rules"
: > "$friction_rules"
grep -v '^#' /scripts/demo-scenario.txt | while read -r class mode speed; do
    case "$mode" in
        walking) base=1000 ;;
        bicycling) base=2000 ;;
        motorized) base=3000 ;;
        *) echo "unknown mode $mode" >&2; exit 1 ;;
    esac
    echo "$class = $(( (base + speed) * 1000 ))" >> "$speed_rules"
    # R: (1 / (speed / 3.6)) * mapResol, printed by paste0 (15 significant digits).
    friction=$(awk -v s="$speed" -v r="$res" 'BEGIN { printf "%.15g", (1 / (s / 3.6)) * r }')
    echo "$class:$class:$friction:$friction" >> "$friction_rules"
done
r.reclass --quiet --overwrite input=rLandCoverMerged__demo output=oracle_speed rules="$speed_rules"
r.recode --quiet --overwrite input=rLandCoverMerged__demo output=oracle_friction rules="$friction_rules"
r.out.gdal -c -m --quiet --overwrite input=oracle_friction \
    output="$out/oracle/friction.tif" format=GTiff createopt=COMPRESS=DEFLATE
r.info -g oracle_friction | grep datatype > "$out/oracle/friction-datatype.txt"
cp /scripts/demo-scenario.txt /scripts/travel-time-cases.txt "$out/"

grep -v '^#' /scripts/travel-time-cases.txt | while read -r name method direction neighbourhood limit; do
    flags=""
    [ "$neighbourhood" = 16 ] && flags="${flags}k"
    if [ "$method" = anisotropic ]; then
        [ "$direction" = towards_facilities ] && flags="${flags}t"
        r.walk.accessmod -s${flags} --quiet --overwrite \
            elevation=rDem__dem@PERMANENT friction=oracle_speed \
            output=oracle_tt nearest=oracle_nearest start_points=vFacility__demo \
            memory=1000 max_cost="$limit"
    else
        if [ -n "$flags" ]; then set -- "-$flags"; else set --; fi
        r.cost "$@" --quiet --overwrite input=oracle_friction \
            output=oracle_tt nearest=oracle_nearest start_points=vFacility__demo \
            memory=1000 max_cost="$limit"
    fi
    r.out.gdal -c -m --quiet --overwrite input=oracle_tt type=Float64 \
        output="$out/oracle/$name-travel_time_seconds.tif" format=GTiff createopt=COMPRESS=DEFLATE
    r.out.gdal -c -m --quiet --overwrite input=oracle_nearest type=Int32 \
        output="$out/oracle/$name-nearest.tif" format=GTiff createopt=COMPRESS=DEFLATE
done
