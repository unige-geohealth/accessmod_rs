#!/bin/sh
# Runs inside the pinned AccessMod 5 image, in a GRASS session on the demo
# mapset (see export.sh). Exports the inputs and outputs of the AccessMod 5
# toolbox "Adjust population distribution" on the demo project, following
# modules/amPopulationBarrierCorrection/helper.R:
#
# - partial population: population where the merged land cover is not null;
# - per zone (v.rast.stats, sum): full and partial population;
# - ratio = full (or known) population / partial population;
# - output = partial population * zone ratio (v.to.rast use=attr).
#
# Two cases: population estimated from the raster (`estimated`), and known
# per zone (`known`, field pop_known added to a copy of the zones).
set -eu

out=/out/population-correction/demo
mkdir -p "$out/input" "$out/oracle"
g.region raster=rDem__dem@PERMANENT

r.out.gdal -c -m --quiet --overwrite input=rDem__dem@PERMANENT \
    output="$out/input/dem.tif" format=GTiff type=Int32 createopt=COMPRESS=DEFLATE
r.out.gdal -c -m --quiet --overwrite input=rLandCoverMerged__demo \
    output="$out/input/land_cover.tif" format=GTiff type=Int32 createopt=COMPRESS=DEFLATE
r.out.gdal -c -m --quiet --overwrite input=rPopulation__demo_patients \
    output="$out/input/population.tif" format=GTiff type=Float32 createopt=COMPRESS=DEFLATE

# Zones with a known population per zone (arbitrary, for the known case).
g.copy --quiet --overwrite vector=vZone__demo,oracle_zones
v.db.addcolumn --quiet map=oracle_zones columns="pop_known double precision"
v.db.update --quiet map=oracle_zones column=pop_known query_column="cat * 10000"
# FlatGeobuf: CRS WKT and no timestamps (see travel-time-demo.sh).
rm -f "$out/input/zones.fgb"
v.out.ogr --quiet input=oracle_zones output="$out/input/zones.fgb" format=FlatGeobuf

r.mapcalc --quiet --overwrite expression="oracle_part = \
if(isnull(rLandCoverMerged__demo), null(), rPopulation__demo_patients)"

# correct CASE FULL_COLUMN
correct() {
    case=$1
    full=$2
    g.copy --quiet --overwrite vector=oracle_zones,oracle_zones_$case
    v.db.addcolumn --quiet map=oracle_zones_$case columns="am_pop_ratio double precision"
    v.rast.stats --quiet map=oracle_zones_$case raster=rPopulation__demo_patients \
        column_prefix=am_pop_full method=sum
    v.rast.stats --quiet map=oracle_zones_$case raster=oracle_part \
        column_prefix=am_pop_part method=sum
    [ "$full" = estimated ] && column=am_pop_full_sum || column=pop_known
    v.db.update --quiet map=oracle_zones_$case layer=1 column=am_pop_ratio \
        query_column="$column/am_pop_part_sum"
    v.to.rast --quiet --overwrite input=oracle_zones_$case output=oracle_ratio_$case \
        use=attr attribute_column=am_pop_ratio
    r.mapcalc --quiet --overwrite expression="oracle_pop_$case = oracle_part * oracle_ratio_$case"
    r.out.gdal -c -m --quiet --overwrite input=oracle_pop_$case type=Float64 \
        output="$out/oracle/population-$case.tif" format=GTiff createopt=COMPRESS=DEFLATE
    v.db.select --quiet map=oracle_zones_$case separator=comma \
        columns=cat,am_pop_full_sum,am_pop_part_sum,am_pop_ratio,pop_known \
        > "$out/oracle/zones-$case.csv"
}

correct estimated estimated
correct known known
