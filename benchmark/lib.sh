# Shared helpers of the benchmark scripts, sourced by benchmark/<module>/run.sh
# (POSIX sh). Environment:
#   REPEAT  runs per measurement, the median is reported (default 3)
#   IMAGE   AccessMod 5 image (default: the pinned oracle image)

IMAGE=${IMAGE:-fredmoser/accessmod:5.9.2-beta.6}
PLATFORM=linux/arm64
REPEAT=${REPEAT:-3}

root=$(cd "$(dirname "$0")/../.." && pwd)
bench="$root/benchmark"
bin="$root/target/release"

# Build the CLI and the synthetic data example (release).
build() {
    (cd "$root" && cargo build --quiet --release -p accessmod-cli &&
        cargo build --quiet --release -p accessmod-io --examples)
}

# Fresh scratch directory for one module: _shared/benchmark/<module>.
scratch() {
    dir="$root/_shared/benchmark/$1"
    rm -rf "$dir"
    mkdir -p "$dir"
    echo "$dir"
}

# Median of the numbers on stdin, one per line.
median() {
    sort -n | awk '{ v[NR] = $1 } END {
        if (NR == 0) { print "-"; exit }
        if (NR % 2) print v[(NR + 1) / 2]; else printf "%.3f\n", (v[NR / 2] + v[NR / 2 + 1]) / 2
    }'
}

# measure <stdout file> <stderr file> command...: run the command once and
# print "<wall seconds> <peak RSS MB>".
measure() {
    out=$1 err=$2
    shift 2
    case $(uname) in
        Darwin)
            /usr/bin/time -l "$@" > "$out" 2> "$err"
            awk '/ real / { s = $1 } /maximum resident set size/ { r = $1 / 1048576 }
                END { printf "%.2f %.0f\n", s, r }' "$err" ;;
        *)
            /usr/bin/time -v "$@" > "$out" 2> "$err"
            awk -F': ' '/Elapsed \(wall clock\)/ { n = split($2, p, ":"); s = 0
                    for (i = 1; i <= n; i++) s = s * 60 + p[i] }
                /Maximum resident set size/ { r = $2 / 1024 }
                END { printf "%.2f %.0f\n", s, r }' "$err" ;;
    esac
}

# repeat_rs <dir> <label> <output dir> command...: run an accessmod-rs
# command $REPEAT times, <output dir> (that of its job) removed before each
# run, and print "<median seconds> <median peak RSS MB>". Files:
# <dir>/<label>-<k>.{json,log}; the JSON report of the last run is
# <dir>/<label>.json.
repeat_rs() {
    dir=$1 label=$2 output=$3
    shift 3
    k=1
    : > "$dir/$label.runs"
    while [ "$k" -le "$REPEAT" ]; do
        rm -rf "$output"
        measure "$dir/$label-$k.json" "$dir/$label-$k.log" "$@" >> "$dir/$label.runs"
        k=$((k + 1))
    done
    cp "$dir/$label-$REPEAT.json" "$dir/$label.json"
    printf '%s %s\n' "$(cut -d' ' -f1 "$dir/$label.runs" | median)" \
        "$(cut -d' ' -f2 "$dir/$label.runs" | median)"
}

# am5 <in dir> <out dir> <script under benchmark/>: run a script in a GRASS
# session of a fresh location built from <in>/dem.tif, in the AccessMod 5
# image. The script sees /in (read-only), /out and /bench (this directory).
am5() {
    docker run --rm --platform "$PLATFORM" -e REPEAT="$REPEAT" -e AM5_JOBS="${AM5_JOBS:-1}" \
        -e AM5_MEMORY_MB="${AM5_MEMORY_MB:-2000}" \
        -v "$bench:/bench:ro" -v "$1:/in:ro" -v "$2:/out" \
        --entrypoint sh "$IMAGE" -c \
        "grass -c /in/dem.tif -e /tmp/db/loc >/dev/null 2>&1 &&
         grass /tmp/db/loc/PERMANENT --exec sh /bench/$3" \
        > "$2/log.txt" 2>&1 || { echo "error: AccessMod 5 run failed, see $2/log.txt" >&2; exit 1; }
}

# am5_session <out dir> <R script under the repository> [args...]: run an
# R script in the AccessMod 5 image on its demo project $REPEAT times, each
# in a fresh container: within one R session AccessMod 5 caches results, so
# a repeated analysis can skip its work. The script loads global.R, writes
# the seconds of that to /out/load.txt and those of its analysis to
# /out/seconds.txt (see benchmark/am5/replay.R); they are appended to
# <out dir>/load-all.txt and <out dir>/seconds.txt; <out dir>/load.txt is
# the median load. The other files of the last run stay in <out dir>/run.
am5_session() {
    out=$1 script=$2
    shift 2
    : > "$out/seconds.txt"
    : > "$out/load-all.txt"
    k=1
    while [ "$k" -le "$REPEAT" ]; do
        # A fresh directory per run: one removed and made again under the same
        # name may not be writable through the Docker bind mount.
        mkdir -p "$out/run-$k"
        docker run --rm --platform "$PLATFORM" -e TZ=UTC -e REPEAT=1 \
            -v "$root:/repo:ro" -v "$out/run-$k:/out" \
            --entrypoint sh "$IMAGE" -c \
            "echo UTC > /etc/timezone; cd /app && Rscript /repo/$script $*" \
            > "$out/log-$k.txt" 2>&1 ||
            { echo "error: AccessMod 5 run failed, see $out/log-$k.txt" >&2; exit 1; }
        cat "$out/run-$k/seconds.txt" >> "$out/seconds.txt"
        cat "$out/run-$k/load.txt" >> "$out/load-all.txt"
        k=$((k + 1))
    done
    rm -rf "$out/run"
    mv "$out/run-$REPEAT" "$out/run"
    median < "$out/load-all.txt" > "$out/load.txt"
}

# am5_replay <out dir> <replay config under the repository>: am5_session
# with an AccessMod 5 replay (benchmark/am5/replay.R).
am5_replay() {
    am5_session "$1" benchmark/am5/replay.R "/repo/$2"
}

# Value of a numeric JSON field (first match) in a file.
json_number() {
    sed -n "s/.*\"$1\": \([0-9.e+-]*\).*/\1/p" "$2" | head -1
}

# Report header: title, command, machine and versions.
header() {
    case $(uname) in
        Darwin)
            cpu=$(sysctl -n machdep.cpu.brand_string)
            cpus=$(sysctl -n hw.ncpu)
            ram=$(($(sysctl -n hw.memsize) / 1073741824))
            os="macOS $(sw_vers -productVersion)" ;;
        *)
            cpu=$(sed -n 's/^model name[^:]*: //p' /proc/cpuinfo | head -1)
            cpus=$(nproc)
            ram=$(awk '/MemTotal/ { printf "%.0f", $2 / 1048576 }' /proc/meminfo)
            os="$(uname -sr)" ;;
    esac
    commit=$(cd "$root" && git describe --always --dirty)
    rustc=$(rustc --version | cut -d' ' -f2)
    image_id=$(docker image inspect -f '{{.Id}}' "$IMAGE" | cut -c8-19)
    vm=$(docker info -f '{{.NCPU}} CPUs, {{.MemTotal}}' | awk '{ printf "%s %s %.1f GB", $1, $2, $3 / 1073741824 }')
    cat <<EOF
# $1

Generated by \`$2\` on $(date -u +%Y-%m-%d).

| | |
| --- | --- |
| Machine | $cpu, $cpus CPUs, $ram GB RAM, $os |
| accessmod-rs | $commit, Rust $rustc, GDAL $(gdal-config --version) |
| AccessMod 5 | \`$IMAGE\` ($PLATFORM, image \`$image_id\`), Docker VM: $vm |
| Runs | median of $REPEAT runs per cell; times are wall-clock seconds |

EOF
}

# Grid size in cells, e.g. "4 M", "90 k".
cells() {
    awk -v n="$1" 'BEGIN { if (n >= 1e6) printf "%g M\n", n / 1e6; else printf "%g k\n", n / 1e3 }'
}

# Number rounded to two decimals.
round2() {
    awk -v n="$1" 'BEGIN { printf "%.2f\n", n }'
}
