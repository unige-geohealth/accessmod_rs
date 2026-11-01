# Distribution

Self-contained archives of AccessMod 6: one binary with GDAL, PROJ, GEOS and
SQLite linked statically, the PROJ database and the built web client next to
it. Milestone 4 of [`plan.md`](plan.md). Status (2026-10-30): archives
build in CI for the five targets (Linux x86_64 and arm64, macOS arm64 and
x86_64, Windows x86_64; 17–20 MB each, `doctor` passing); macOS arm64 is
also tested end to end locally. macOS notarization waits for the Apple
secrets in this repository.

## Archive

```sh
sh scripts/package.sh [target]     # default: the host target
```

`dist/accessmod-<version>-<target>.tar.gz` (`.zip` on Windows) and its
`.sha256`:

```text
accessmod                  the binary
share/proj/proj.db         PROJ database
share/accessmod/web/       built web client
LICENSE, COPYING           LGPL-3.0-or-later
THIRD-PARTY-LICENSES.md    native libraries and every Rust crate linked, with licences
licenses/                  GDAL, PROJ and GEOS licence texts
README.txt                 quick start
```

The binary finds `share/` next to itself (`accessmod_io::install`): PROJ
uses the shipped database unless `PROJ_DATA` or `PROJ_LIB` is set, and
`accessmod serve` serves the shipped client unless `--web-dir` is given.
`accessmod doctor` reports the drivers, GEOS and which PROJ database is used.

## Bundled GDAL

The cargo feature `bundled` (`accessmod-io`, `accessmod-cli`) enables
`gdal-sys/bundled`: GDAL 3.12 is built from source by the `gdal-src` crate,
with PROJ (`proj-sys`, bundled), GEOS (`geos-src`, static) and SQLite
(`libsqlite3-sys`, bundled), all linked statically. Development builds keep
the system GDAL (`gdal-config` on `PATH`). Building needs cmake and a C/C++
compiler; a clean build of the CLI takes about 3 min on an Apple M1 Pro.

Only the drivers the code uses are built: GTiff, MEM, HFA (ERDAS Imagine,
the AccessMod 5 sample data), XYZ, ESRI Shapefile, GPKG (SQLite),
FlatGeobuf, GeoJSON, CSV, plus GEOS for geometry operations (optimization
subtracts catchments). Consequences:

- XLSX workbooks are read in Rust (`calamine`), not by GDAL, in every
  build: `gdal-src` has no XLSX driver. The demo tables are byte-identical
  to the former GDAL reading.
- One SQLite for the project database (`rusqlite`) and GeoPackage, the one
  GDAL uses: the system library with the system GDAL, the bundled one with
  the feature `bundled`. On Linux a second, bundled SQLite in the binary
  took over `libgdal`'s calls (ELF symbol interposition) and crashed
  GeoPackage writes on GDAL 3.8. `rusqlite` is held at 0.37, whose
  `libsqlite3-sys` 0.35 is the newest `gdal-src` accepts (a `links` key
  admits one version per build).
- System GDAL 3.8 of Ubuntu 24.04 (linked with spatialite) aborts with
  "double free or corruption" when threads create GeoPackages at once, even
  from Python in separate files (reproduced in Docker, 9 runs out of 10).
  AccessMod only reads GeoPackages, which is safe; the test fixtures create
  them one at a time. The bundled GDAL has no spatialite.
- The bundled GDAL opens no VRT raster: an upload holding a VRT cannot make
  it read another file (`check_self_contained` keeps refusing it where the
  system GDAL would read it).

Linked libraries of the macOS arm64 binary (`otool -L`): only system
libraries and frameworks (libSystem, libc++, libiconv, Foundation, IOKit,
CoreFoundation).

## Results (macOS arm64, 2026-10-30)

| Check | Result |
| --- | --- |
| Binary | 36 MB; archive 18 MB |
| `cargo test --workspace --features accessmod-io/bundled` | 271 passed |
| `doctor` in `env -i` (no Homebrew), from the unpacked archive | ok: every driver, GEOS, shipped PROJ database |
| `project demo` from the archive | ok |
| End-to-end tests (`ACCESSMOD_E2E_BIN=…/accessmod npm run e2e`) | 8 passed, client served from `share/` |
| Tutorial travel-time job, bundled against system build | every output byte-identical (`run.json` differs in timings and memory only) |
| Signature (`codesign --options runtime --timestamp`) | valid, Developer ID |
| Notarization | in CI, once the secrets are in the repository |

## Signing and notarization (macOS)

Done in CI (`.github/workflows/release.yml`) from the repository secrets,
the names AccessMod 5 uses: `APPLE_CERTIFICATE_BASE64` and
`APPLE_CERTIFICATE_PASSWORD` (Developer ID Application certificate, imported
into a temporary keychain), `APPLE_API_KEY_BASE64`, `APPLE_API_KEY_ID` and
`APPLE_API_ISSUER` (App Store Connect API key for `notarytool`). Without
them the archives are built unsigned. `scripts/package.sh` reads the
environment, so a local run can sign too:

```sh
ACCESSMOD_SIGN_IDENTITY="Developer ID Application: …" \
ACCESSMOD_NOTARY_PROFILE=accessmod \
sh scripts/package.sh
```

`ACCESSMOD_NOTARY_PROFILE` names a keychain profile made once with
`xcrun notarytool store-credentials`; CI passes an App Store Connect API
key instead (`ACCESSMOD_NOTARY_KEY`, `_KEY_ID`, `_ISSUER`, as the
AccessMod 5 secrets `APPLE_API_KEY_*`). A bare binary cannot be stapled:
Gatekeeper checks its ticket online on first run.

## Licences

GDAL and PROJ are MIT, SQLite public domain, GEOS LGPL-2.1. GEOS is linked
statically: its licence requires that users can relink the program with
another GEOS, which holds because this program's source is published under
LGPL-3.0-or-later. **Binaries are distributed only with the source
available** (the repository is private today: keep releases private too
until it is public).

## Next

- Windows: PROJ builds `proj.db` with the `sqlite3` command (installed on
  the runner); the build itself needs no other change. Windows code signing
  needs a certificate (none yet).
- End-to-end tests against the Linux and Windows archives in CI.
- Desktop installers wrap this archive: [`desktop.md`](desktop.md).

## Online basemaps

The interactive map offers MapTiler basemaps when `accessmod serve` has a
key: `MAPTILER_API_KEY` in its environment, else the one embedded at build
time. The release workflow embeds the repository secret
`MAPTILER_API_KEY` (a public browser key, as in AccessMod 5). Restrict it
in MapTiler to the origins the client uses: `http://localhost:*` and
`http://127.0.0.1:*` (`serve` and the desktop window), plus a country
server's address. In development, `.env` at the repository root (ignored;
template `.env.demo`) holds it for `npm run dev:server`. Without a key
or a network, the offline schematic basemap stays.
