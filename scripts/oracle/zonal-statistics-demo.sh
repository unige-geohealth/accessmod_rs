#!/bin/sh
# Runs inside the pinned AccessMod 5 image, in a GRASS session on the demo
# mapset (see export.sh). Exports the inputs and outputs of the AccessMod 5
# zonal statistics (tools/R/amAnalysisZonal.R) on the demo project:
#
# - zones rasterised with v.to.rast use=attr (id cat, label admin_name);
# - per zone: population sum, and population where 0 <= travel time < limit
#   (minutes, the stored AccessMod 5 travel time), through r.univar zones.
set -eu

out=/out/zonal-statistics/demo
mkdir -p "$out/input" "$out/oracle"
g.region raster=rDem__dem@PERMANENT

r.out.gdal -c -m --quiet --overwrite input=rDem__dem@PERMANENT \
    output="$out/input/dem.tif" format=GTiff type=Int32 createopt=COMPRESS=DEFLATE
r.out.gdal -c -m --quiet --overwrite input=rTravelTime__test \
    output="$out/input/travel_time_minutes.tif" format=GTiff type=Int32 createopt=COMPRESS=DEFLATE
r.out.gdal -c -m --quiet --overwrite input=rPopulation__demo_patients \
    output="$out/input/population.tif" format=GTiff type=Float32 createopt=COMPRESS=DEFLATE
# FlatGeobuf: CRS WKT and no timestamps (see travel-time-demo.sh).
rm -f "$out/input/zones.fgb"
v.out.ogr --quiet input=vZone__demo output="$out/input/zones.fgb" format=FlatGeobuf

v.to.rast --quiet --overwrite input=vZone__demo output=oracle_zones type=area \
    use=attr attribute_column=cat label_column=admin_name
r.univar -t --quiet map=rPopulation__demo_patients zones=oracle_zones separator=comma \
    > "$out/oracle/population.csv"
for limit in 30 60; do
    r.mapcalc --quiet --overwrite expression="oracle_covered = \
if(rTravelTime__test >= 0 && rTravelTime__test < $limit, rPopulation__demo_patients, null())"
    r.univar -t --quiet map=oracle_covered zones=oracle_zones separator=comma \
        > "$out/oracle/covered-$limit.csv"
done
