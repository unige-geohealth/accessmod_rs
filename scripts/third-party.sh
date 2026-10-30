#!/bin/sh
# THIRD-PARTY-LICENSES.md of a release archive: the native libraries built
# into the binary, then every Rust crate it links, with its licence.
set -e
cd "$(dirname "$0")/.."
cat <<'HEAD'
# Third-party software in this archive

Native libraries, built from source and linked statically (texts in
`licenses/`):

| Library | Licence |
| --- | --- |
| GDAL 3.12 (with the libraries it builds internally: libtiff, libgeotiff, json-c, zlib, …) | MIT and others, see `licenses/GDAL.txt` |
| PROJ 9 | MIT, `licenses/PROJ.txt` |
| GEOS | LGPL-2.1, `licenses/GEOS.txt`; this program's source is published under LGPL-3.0-or-later, so it can be relinked with another GEOS |
| SQLite | public domain |

Rust crates:

| Crate | Version | Licence |
| --- | --- | --- |
HEAD
cargo tree -q -p accessmod-cli --features bundled -e normal --prefix none --format '{p}|{l}' |
	sed 's/ (\*)$//; s/ (proc-macro)//; s/ (\/[^)]*)//' |
	sort -u |
	awk -F'|' '{ split($1, a, " v"); printf "| %s | %s | %s |\n", a[1], a[2], ($2 == "" ? "see the crate" : $2) }'
