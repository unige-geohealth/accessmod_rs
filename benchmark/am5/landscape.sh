# Sourced inside the AccessMod 5 image, in a GRASS session (see am5 in
# benchmark/lib.sh): imports /in/dem.tif, /in/land_cover.tif and
# /in/facilities.gpkg, and builds the speed map as amCreateSpeedMap does
# (speed = (mode raster value + km/h) * 1000, r.reclass) from
# /bench/scenario.txt. Defines `now` (seconds, from /proc/uptime).
r.in.gdal --quiet -o input=/in/dem.tif output=dem
r.in.gdal --quiet -o input=/in/land_cover.tif output=lc
v.in.ogr --quiet -o input=/in/facilities.gpkg output=hf
g.region raster=dem
: > /tmp/speed.rules
grep -v '^#' /bench/scenario.txt | while read -r class mode speed; do
    case "$mode" in
        walking) base=1000 ;; bicycling) base=2000 ;; motorized) base=3000 ;;
    esac
    echo "$class = $(( (base + speed) * 1000 ))" >> /tmp/speed.rules
done
r.reclass --quiet input=lc output=speed rules=/tmp/speed.rules

now() { cut -d' ' -f1 /proc/uptime; }
# elapsed <start>: seconds since <start>.
elapsed() { awk -v a="$1" -v b="$(now)" 'BEGIN { printf "%.2f\n", b - a }'; }
