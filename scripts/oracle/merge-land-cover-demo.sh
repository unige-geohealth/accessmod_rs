#!/bin/sh
# Runs inside the pinned AccessMod 5 image, in a GRASS session on the demo
# mapset (see export.sh). Exports the inputs of the AccessMod 5 "Merge land
# cover" toolbox on the demo project, every stack item as AccessMod 5
# rasterises it, and the merged land cover with and without bridge cleaning.
#
# Steps follow modules/amAnalysisMergeLandCover/amServer.R and
# tools/R/amStackBarrier.R, tools/R/amBridgeTools.R:
# - roads: one stack item per road class (v.extract on the class field),
#   rasterised with `v.to.rast -d type=line` and value class + 1000;
# - barriers: value 1, `v.to.rast -d type=line` for lines,
#   `v.to.rast type=area` for areas;
# - merge: barrier cells are masked out of every other item, then
#   `r.patch` keeps the first non-null item in stack order;
# - bridges: a road cell whose merged diagonal neighbour is non-null while
#   the two cells sharing that corner are null is set to null.
set -eu

out=/out/merge-land-cover/demo
mkdir -p "$out/input" "$out/oracle"
g.region raster=rDem__dem@PERMANENT

export_int() {
    r.out.gdal -c -m --quiet --overwrite input="$1" output="$2" \
        format=GTiff type=Int32 createopt=COMPRESS=DEFLATE
}

export_int rDem__dem@PERMANENT "$out/input/dem.tif"
export_int rLandCover__demo "$out/input/land_cover.tif"
# FlatGeobuf: CRS WKT and no timestamps (see travel-time-demo.sh).
for pair in vRoad__demo:roads vBarrier__demo_river:river vBarrier__demo_wetland:wetland; do
    name=${pair%%:*}
    file=${pair#*:}
    rm -f "$out/input/$file.fgb"
    v.out.ogr --quiet input="$name" output="$out/input/$file.fgb" format=FlatGeobuf
done

# Stack items.
for class in 1 2 3; do
    v.extract --quiet --overwrite input=vRoad__demo output=oracle_road_tmp \
        where="class_n=$class"
    v.to.rast --quiet --overwrite -d input=oracle_road_tmp output="oracle_road_$class" \
        use=val type=line value=$((class + 1000))
    export_int "oracle_road_$class" "$out/oracle/stack-road-$class.tif"
done
v.to.rast --quiet --overwrite -d input=vBarrier__demo_river output=oracle_barrier_river \
    use=val type=line value=1
v.to.rast --quiet --overwrite input=vBarrier__demo_wetland output=oracle_barrier_wetland \
    use=val type=area value=1
export_int oracle_barrier_river "$out/oracle/stack-barrier-river.tif"
export_int oracle_barrier_wetland "$out/oracle/stack-barrier-wetland.tif"

# merge NAME ITEM... : merge non-barrier items in the given order.
merge() {
    name=$1
    shift
    patch=""
    for item in "$@"; do
        r.mapcalc --quiet --overwrite expression="oracle_tmp_$item = \
if(isnull(oracle_barrier_river) && isnull(oracle_barrier_wetland), $item, null())"
        patch="${patch:+$patch,}oracle_tmp_$item"
    done
    r.patch --quiet --overwrite input="$patch" output="oracle_merged_$name"
    export_int "oracle_merged_$name" "$out/oracle/merged-$name.tif"

    m="oracle_merged_$name"
    roads="!isnull(oracle_road_1) || !isnull(oracle_road_2) || !isnull(oracle_road_3)"
    diag="(isnull($m[0,-1]) && !isnull($m[1,-1]) && isnull($m[1,0])) || \
(isnull($m[0,1]) && !isnull($m[1,1]) && isnull($m[1,0])) || \
(isnull($m[-1,0]) && !isnull($m[-1,1]) && isnull($m[0,1])) || \
(isnull($m[0,-1]) && !isnull($m[-1,-1]) && isnull($m[-1,0]))"
    r.mapcalc --quiet --overwrite expression="oracle_bridges_$name = \
if($roads, if($diag, 1, null()), null())"
    r.mapcalc --quiet --overwrite expression="oracle_cleaned_$name = \
if(!isnull(oracle_bridges_$name), null(), $m)"
    export_int "oracle_bridges_$name" "$out/oracle/bridges-$name.tif"
    export_int "oracle_cleaned_$name" "$out/oracle/merged-cleaned-$name.tif"
}

merge roads_first oracle_road_1 oracle_road_2 oracle_road_3 rLandCover__demo
merge roads_reversed oracle_road_3 oracle_road_2 oracle_road_1 rLandCover__demo
