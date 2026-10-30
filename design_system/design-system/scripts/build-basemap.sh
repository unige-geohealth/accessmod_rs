#!/bin/sh
# Build the offline schematic basemap (public/basemap/countries.pmtiles).
#
#   scripts/build-basemap.sh <countries.geojson|.gpkg|.shp> [maxzoom]
#
# Input: country polygons in WGS84 with a `name` attribute (the demo uses a
# Natural Earth 1:110m derivative, public domain). For production pick a
# finer source (Natural Earth 1:10m, geoBoundaries ADM0) and raise maxzoom.
# Output layer: `countries` (polygons); MapFrame draws fill + outline from it.
set -eu
src=${1:?usage: build-basemap.sh <countries file> [maxzoom]}
maxzoom=${2:-6}
out="$(dirname "$0")/../public/basemap/countries.pmtiles"
mkdir -p "$(dirname "$out")"
tippecanoe -q --force -o "$out" -l countries -Z0 -z"$maxzoom" \
  --detect-shared-borders --no-tile-size-limit -y name "$src"
pmtiles show "$out" | head -8
