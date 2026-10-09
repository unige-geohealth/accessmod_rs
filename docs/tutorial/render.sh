#!/bin/sh
# Render the tutorial figures from data/ and data/out/ into img/.
# Needs the GDAL command-line tools (gdaldem, gdal_translate, gdal_rasterize).
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p img

cat > "$tmp/land_cover.txt" <<'C'
0 168 205 235
1 214 92 92
2 205 222 150
3 76 140 80
9 66 120 200
1001 30 30 30
1002 150 110 70
nv 255 255 255
C
# Travel time in seconds: 0, 15, 30, 60, 120, 240 minutes.
cat > "$tmp/travel_time.txt" <<'C'
0 26 152 80
900 145 207 96
1800 217 239 139
3600 254 224 139
7200 252 141 89
14400 215 48 39
nv 255 255 255
C
cat > "$tmp/nearest.txt" <<'C'
1 141 160 203
2 252 141 98
3 102 194 165
nv 255 255 255
C

# Facilities drawn as black squares on an RGB copy, then upscaled to PNG.
figure() { # name source colours [exact]
    gdaldem color-relief -q ${4:+-exact_color_entry} "$2" "$3" "$tmp/$1.tif"
    gdal_rasterize -q -b 1 -b 2 -b 3 -burn 0 -burn 0 -burn 0 \
        -dialect SQLite -sql "SELECT ST_Buffer(geom, 300) FROM facilities" \
        data/facilities.gpkg "$tmp/$1.tif"
    gdal_translate -q -of PNG -outsize 900 600 -r nearest "$tmp/$1.tif" "img/$1.png"
}

cat > "$tmp/dem.txt" <<'C'
150 40 110 60
350 170 200 110
550 225 200 140
750 160 110 80
nv 255 255 255
C
figure dem data/dem.tif "$tmp/dem.txt"
figure land_cover data/land_cover.tif "$tmp/land_cover.txt" exact
figure travel_time data/out/travel_time_seconds.tif "$tmp/travel_time.txt"
figure nearest data/out/nearest_facility.tif "$tmp/nearest.txt" exact
rm -f img/*.aux.xml
echo "figures written to $(pwd)/img"
