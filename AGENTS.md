# AGENTS.md — accessmod-rs

Guide for humans and coding agents (Claude Code, Codex, …) working in this
repository. Read [`docs/plan.md`](docs/plan.md) first: it is the source of
truth for architecture, contracts and milestones. The interface (project
layer, `accessmod serve`, the client) follows [`docs/ui.md`](docs/ui.md);
its look follows [`design_system/docs/DESIGN.md`](design_system/docs/DESIGN.md).
Code style, Rust and TypeScript, is in [`STYLE.md`](STYLE.md).

## Ground rules

- **Language**: everything is in English — code, comments, docs, plans,
  commit messages, issues.
- **Units**: internal values and canonical outputs are SI — **seconds** and
  **metres** (km/h only as scenario input speeds). Minutes, kilometres and
  rounding exist only at the presentation edge: reports, summaries, display,
  compatibility exports.
- **Memory**: never assume the grid fits in RAM. Targets range from small
  machines (2 GB RAM, 2 CPU) to compute servers (40 GB RAM, 12 CPU). All
  raster work goes through the tiled, disk-backed storage with an explicit
  memory budget (see `docs/plan.md`, "Storage and memory").
- **Tooling dependencies**: building, testing and generating artefacts of the
  Rust workspace (e.g. the bicycle LUT, the JSON Schemas) need only the Rust
  toolchain; `cargo build --workspace` never runs Node. The front end
  (`design_system/`, then the client) needs pinned Node LTS and npm,
  nothing else; its Node scripts are front-end dev tools (gallery, visual
  report, contrast, AM5 capture). Helper scripts are Rust
  (`cargo xtask …`) or POSIX `sh`. No R; no Python in the build or test path.
  Docker is needed only to export the AccessMod 5 oracle.
- **Interface state**: no cascades. Typed messages, pure `update`, derived
  values as selectors, no effect that sets state from other state
  ([`docs/ui.md`](docs/ui.md), "State"). Reviewers treat a violation as a
  defect.
- **Jobs are notified**: every job, whatever starts it, gets a notice when
  queued and when it ends, from the server events; a form never tells a
  job's state itself ([`docs/ui.md`](docs/ui.md), "Notifications").
- **Never crash**: input from users, files and the network is hostile. The
  server and every command return errors, never panic; a country server
  runs unattended. A crash fix ships with the test that panicked before it.
- **Every action is a command** in the server registry (parameter schema,
  role, `Result`); the HTTP API, the CLI and a future MCP dispatch it
  ([`docs/ui.md`](docs/ui.md), "Commands").
- **National scale**: a preview, a table or a legend never reads a whole
  grid or layer; overviews, statistics, tiles and paged queries do
  ([`docs/ui.md`](docs/ui.md), "Scale").

## Commits

- Conventional commits: `feat:`, `fix:`, `refactor:`, `perf:`, `test:`,
  `docs:`, `chore:`, `ci:`; optional scope, e.g. `feat(core): …`.
- **One line only. No body. No `Co-Authored-By` or any other trailer.**
- Commit only what the change needs; never commit `_oracle/`, `_am5/`,
  `_shared/` or `target/`.

## Agents

One implementing agent at a time keeps the whole design in view; parallel
implementations produced more noise than value. Other agents review
(Codex, see "Tooling"); they do not implement in parallel.

- Work in a branch per task, e.g. `claude/<topic>`, branched from `main`.
- Keep each branch self-contained and passing all checks so it can be
  merged as a unit; keep the tree building at every commit, so an
  interrupted session can be resumed.
- Summarise in the final message (or PR description): what was done, what was
  verified, known gaps, and any deviation from `docs/plan.md`.

## Relationship with AccessMod 5

- This repository is independent: it does not vendor, submodule or build
  anything from <https://github.com/unige-geohealth/accessmod>.
- AccessMod 5 is the **numerical oracle**, pinned at tag `5.9.2-beta.6`.
  Where each reference behaviour lives is listed in
  [`docs/reference-accessmod5.md`](docs/reference-accessmod5.md).
- Reimplement from specification and oracle outputs. Do not translate
  AccessMod 5 or GRASS source line by line (GRASS modules are GPL).
- A divergence from AccessMod 5 is allowed only when deliberate, documented in
  `docs/`, and reported in `run.json` warnings when user-visible.

### AccessMod 5 source

A read-only shallow clone of the pinned tag lives in `_am5/accessmod/`
(ignored by git). Create or verify it with:

```sh
sh scripts/fetch-am5.sh
```

- Answer behaviour questions (NoData, neighbours, limits, rounding, LUT, …)
  from this source before asking the user; cite paths relative to
  `_am5/accessmod/`, never absolute or sibling-checkout paths.
- Never edit it, copy code from it or build it; the GPL rule above applies.
- **Vocabulary**: domain names (CLI, job fields, outputs, types, API,
  interface, docs) follow the English names in
  `www/dictionary/classes.json` and `main.json`, e.g.
  *facility* (French *structure*), *land cover*, *cost allocation*. "Source"
  is reserved for solver internals.

## Commands

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo xtask lut [--check]  # regenerate (or verify) the bicycle LUT
sh scripts/check.sh        # the whole gate, as before a commit
npm run release -- <version> [--dry-run]  # scripts/release.sh, from main: gates, version, commit, tag, push (CI publishes)
sh scripts/package.sh [target]  # self-contained archive in dist/ (bundled GDAL, docs/distribution.md)
sh scripts/package-desktop.sh [target]  # + desktop installers (Tauri; .dmg, .deb, .AppImage, setup .exe; docs/desktop.md)
ACCESSMOD_E2E_BIN=$PWD/dist/<name>/accessmod npm run e2e  # e2e against a packaged binary
sh scripts/oracle/export.sh # export AccessMod 5 oracle artefacts into _oracle/ (Docker)
cargo run -p accessmod-cli -- doctor --json
cargo run -p accessmod-cli -- travel-time run --job job.json --json
cargo run -p accessmod-cli -- referral run --job referral.json --json
cargo run -p accessmod-cli -- project init p --dem dem.tif --name Demo
cargo run -p accessmod-cli -- project demo demo   # the demo project (AM5 sample data, tagged demo)
cargo run -p accessmod-cli -- dataset import lc.tif --class rLandCover --tags base --project p
cargo run -p accessmod-cli -- project run travel-time --job job.json --project p   # paths relative to p
sh benchmark/travel-time/run.sh 1000 2000   # benchmark vs AccessMod 5 (Docker), writes report.md
sh benchmark/referral/run.sh 1000 20         # idem; also benchmark/geographic-coverage/, scaling-up/, optimization/
```

Interface (milestone 3):

```sh
cargo xtask schemas [--check]               # JSON Schemas into web/schemas/ (committed)
cargo xtask i18n [--check]                  # design_system catalogue from the AM5 dictionary + own.json (committed)
cargo xtask layers                          # crate dependency rules (core: no GDAL, …)
cargo run -p accessmod-cli -- serve         # API, http://127.0.0.1:5099 (--web-dir: a built client)
npm ci                                      # root npm workspace: @accessmod/ui and @accessmod/web
npm run dev:server                          # serve (debug) on 5099, data in _shared/dev-data, restarted on Rust changes (watchexec); reads .env (MAPTILER_API_KEY)
npm run dev:web                             # client, http://localhost:5173 (proxies /api to dev:server)
npm run dev:gallery                         # design-system gallery and AM5 mock screens, http://localhost:5180
npm run dev                                 # dev:server + dev:web in one terminal; args go to serve
npm run check                               # front-end gate: typecheck, contrast, test, build (both packages)
npm -w @accessmod/ui run dev                # gallery, http://localhost:5180
npm -w @accessmod/ui run typecheck
npm -w @accessmod/ui run contrast           # WCAG AA, both themes
npm -w @accessmod/ui run build
npm -w @accessmod/ui run visual             # with dev running: visual/report.html
npm -w @accessmod/web run dev               # client, http://localhost:5173 (proxies /api to serve on 5099)
npm -w @accessmod/web run typecheck
npm -w @accessmod/web test                  # Vitest: updates, selectors, state machines
npm -w @accessmod/web run build             # web/dist, for serve --web-dir web/dist
npm -w @accessmod/web run e2e               # Playwright on the tutorial data (build first)
```

The workspace needs GDAL ≥ 3.5 and PROJ installed (`gdal-config` on `PATH`).

Build, test, clippy and fmt must all pass before a commit; when
`design_system/`, `web/` or the schemas change, `cargo xtask schemas
--check`, `cargo xtask i18n --check`, the design system's `typecheck`,
`contrast` and `build`, and the client's `typecheck`, `test` and `build`
too (`e2e` when a screen changes).

## Layout

```
crates/accessmod-core   tiled storage, raster primitives, mobility and cost models, solver, referral, merge, population, zonal, capacity, scaling up, optimization, focal operations (no GDAL)
crates/accessmod-io     GDAL/PROJ boundary: GIS toolkit, jobs, analyses, outputs, reports; fixtures (tests); examples/; demo/ (AM5 sample data, embedded)
crates/accessmod-cli    `accessmod` binary (clap): doctor, project, dataset, travel-time, referral, merge-land-cover, adjust-population-distribution, zonal-statistics, geographic-coverage, scaling-up, optimization
crates/accessmod-fetch  HTTPS client and cache of checked downloads, Project Creator source catalogue (no GDAL)
crates/accessmod-server `accessmod serve`: command registry, JSON API + events generic over `Analysis`, accounts and roles, run queue, tiles, embedded front end (feature `web`)
design_system/          AM5 references (screens, DOM, i18n keys, measured styles) and the design system: tokens, components, gallery (React, Tailwind, shadcn/ui)
web/                    the client (@accessmod/web): React screens on @accessmod/ui, TanStack Query, stores; e2e/ (Playwright)
r/                      R package (R6 sessions over the engine, pinned by `Config/accessmod/engine`, set by release.sh; docs/r-package.md); check: `R CMD build r && R CMD check --no-manual accessmod_*.tar.gz` (ACCESSMOD_BIN runs the demo test), docs: `roxygen2::roxygenise("r")`
desktop/                desktop window (Tauri 2), own Cargo workspace: runs `accessmod serve` as a sidecar (docs/desktop.md)
web/schemas/            JSON Schemas generated by `cargo xtask schemas` (types of the client, committed)
package.json            npm workspace: design_system/design-system (@accessmod/ui) and web (@accessmod/web), one lockfile
xtask/                  developer tasks (`cargo xtask lut`, `cargo xtask schemas`, `cargo xtask i18n`)
_oracle/                AccessMod 5 exports (ignored; checksummed manifest committed)
_am5/accessmod/         pinned AccessMod 5 source, read-only (ignored; `scripts/fetch-am5.sh`)
_shared/                local scratch, issue reproductions (ignored)
scripts/                POSIX sh helpers: `oracle/` (export + committed manifest)
benchmark/              per-module benchmarks vs AccessMod 5: scripts, README, generated report.md
docs/                   plan, references, design decisions, tutorial (with data and figures)
```

`accessmod-core` never depends on GDAL, PROJ, a project layout or an external
process. `accessmod-io` never depends on the server or HTTP. `cargo xtask
layers` (milestone 3) checks these rules.

## Reusable GIS toolkit

Analyses are short compositions of shared, low-level GIS building blocks,
the way AccessMod 5 composes GRASS modules (`v.to.rast`, `r.patch`,
`r.mapcalc`). An analysis module holds only its domain rules; the
mechanics live in the toolkit and are written, tested and reviewed once:
determinism, memory budget, cancellation and error messages included.

| Need | Toolkit |
| --- | --- |
| Rasterise lines and polygons, exact cell coverage fractions of a polygon (`polygon_coverage`, as `exactextractr`), 3 × 3 neighbourhoods | `accessmod_core::raster` |
| Distance to source cells, sums over a disc (`r.grow.distance`, `r.buffer`, `r.neighbors -c`) | `accessmod_core::focal` |
| Disk-backed grids under a memory budget | `accessmod_core::store` |
| Open aligned rasters, read windows, blocks of several bands with zones | `accessmod_io::raster` (`for_each_block`) |
| Open a vector layer (filter, CRS, reprojection), burn features (dense lines, or `Burn::Thin` as `v.to.rast` without `-d`) and zones, polygonise cells, sums over polygons weighted by coverage, geometries spooled to disk, write FlatGeobuf, extract features clipped to a polygon | `accessmod_io::vector` (`burn_layer`, `burn_zones`, `polygonize`, `covered_sum`, `GeometrySpool`, `FeatureWriter`, `extract`, `parts_as_multi`, `centroid`) |
| Residual population, band search, catchment polygon, removal (capacity analyses) | `accessmod_io::catchment` |
| DEM grid (or the grid of another raster, `open_raster`, or of an extent on a lattice, `from_extent`), lattice alignment, map ↔ pixel coordinates | `accessmod_io::grid` |
| Projections for a region (UTM, centred Transverse Mercator, Albers, LAEA) and their distance and area distortion over it (Tissot) | `accessmod_io::projection` |
| Rasters from elsewhere onto a grid: reprojection and resampling (nearest, mode, bilinear, sum) of tiles, chunked mosaics without seams; source totals over a grid footprint (exact coverage) | `accessmod_io::warp` (`warp_to_grid`, `source_total`) |
| A project from a region and local sources (recipe, presets, merge, manifest) | `accessmod_io::creator` |
| Job files, path resolution, schema version, input checks | `accessmod_io::job` |
| Memory plan and execution settings | `accessmod_io::resources` |
| Staging directory, GeoTIFF blocks, CSV, `finish` | `accessmod_io::output` |
| Raster overviews in an external `.ovr`, statistics (range, histogram, classes) | `accessmod_io::preview` |
| Map previews: raster tiles in Web Mercator (PNG, through the overviews), GeoJSON in WGS 84, values at a point (`PointSampler`), bounds | `accessmod_io::tiles` |
| RFC 4180 CSV reading (quoted fields, bounded records) | `accessmod_io::output::read_csv` |
| Paged attribute rows of a vector layer or a table (bounded pages, filter, total, rasters sampled at points) | `accessmod_io::rows` |
| Data classes (AccessMod 5 identifiers), `x-accessmod-class` on job fields | `accessmod_io::class` |
| `run.json` (`Report`, `Execution`), fingerprints, warnings | `accessmod_io::report` |
| `prepare` / `run` contract driven by the CLI and `accessmod serve`, progress (`Monitor`), registry of analyses (`for_each`) | `accessmod_io::analysis` |
| Test inputs, oracle paths, running a job | `accessmod_io::fixtures` (feature `fixtures`), `accessmod-core/tests/common` |
| Project: SQLite metadata, import on the project grid, datasets (classes, tags, summaries), saved jobs, runs registering their outputs, paths confined to the project | `accessmod_io::project` |

Rules:

- No copy of a helper in an analysis, a command or a test. Before writing
  one, look in the toolkit; if it is not domain-specific, add it there with
  its own test and use it from every caller.
- A new analysis implements `Analysis`; its CLI command is
  `exec::job_command::<A>` unless it needs direct parameters. The server
  exposes it by registration alone; a bespoke screen talks to the generic
  API until it needs more.
- Reviewers treat duplicated logic as a defect.

## Local scratch

Use `_shared/<topic>/` for investigations, throwaway scripts and comparison
outputs (ignored by git). Keep outputs small and readable when they may be
pasted into issues.

## Oracle data

AccessMod 5 exports live under `_oracle/` (ignored). Each export is produced
by a committed script run against the pinned AccessMod 5 Docker image and
described by a committed manifest (files, SHA-256, oracle version). Tests that
need `_oracle/` skip with an explicit message when it is absent; they never
pass vacuously.

## Findings and pitfalls

Things learned the hard way; check here before re-investigating.

### AccessMod 5 oracle

- Run the image with `--platform linux/arm64`: binaries and image IDs differ
  per platform, and the manifest records the arm64 image ID.
- Replays (`Rscript` + `amAnalysisReplayExec`) fail unless `TZ` is set and
  `/etc/timezone` exists (`echo UTC > /etc/timezone`). The export directory
  must exist beforehand, and the example configs have no `output` list, so
  nothing is exported: read result tables from the project's
  `sqlite.db` instead (see `scripts/oracle/referral-demo.R`).
- Loading `global.R` alone takes about 6 s; it dominates small replays.
- Byte-stable exports: a GeoPackage embeds timestamps (manifest breaks),
  GeoJSON cannot carry a CRS without an EPSG code (the demo's UTM 36 has
  none). Use FlatGeobuf for vectors; note it reorders features (spatial
  index), so compare by identifier, never by row order.
- The bicycle LUT is not shipped as a file: it is extracted from the
  compiled `r.walk.accessmod`; our table is bit-identical.
- Isotropic friction from `r.recode` is FCELL (Float32): isotropic times
  differ by up to 2⁻²⁴ relative, by construction.
- Referral: AM5 rounds times up in the requested unit (use seconds for
  comparisons), drops self pairs, keeps closest-time ties, and measures
  distances with `r.path` from the facility points (ours: cell centres;
  up to one cell diagonal apart).
- Merge land cover: AM5 rasterises lines with `v.to.rast -d`. Our
  half-open "all touched" rule differs on 1 road and 1 river cell of the
  demo (`gdal_rasterize -at`: 7), so the merged land cover differs on 2 of
  44 492 cells; areas (`type=area`, cell centre) and bridges are identical.
  Barriers work by position in AM5 (`processStack`): the first is a mask
  for every item, each later one masks the items below it. The first oracle
  script masked every item with every barrier and hid that; reproduce the
  module's sequence, not a summary of it.
- Adjust population distribution: equal to AM5 within 6e-15 relative on the
  demo (summation order). The demo merged land cover has few barrier cells,
  so the estimated case moves only 41 people; the known case (field
  `pop_known` added by the oracle script) exercises the ratios.
- Zonal statistics: AM5 covers `t < L` on whole minutes (rounded up); ours
  `t <= L` in seconds. On the same minutes, AM5 `L` equals ours `(L - 1) x
  60 s`; with that the demo sums match to 1e-9.
- Geographic coverage works on whole-minute bands (`ceil(t / 60)`); keep
  `band_s = 60` for parity. Residual init uses the null-aware `&&&`: people
  <= 0 give 0 even on barriers. AM5 catchment shapefiles carry a DBF date:
  export them with `ogr2ogr -nomd`.
- Scaling up: ties between candidate cells are frequent (integer
  suitability), so AM5 arithmetic is reproduced: 15-digit min / max / weights,
  integer rescale of minute travel times. The
  demo replay references `rPopulation__cor_pop`, absent from the image
  (`rPopulation__demo_patients` is used). GRASS `v.to.rast` without `-d`
  steps each segment along its major axis through the cells between its end
  points (`raster::thin_line_cells`); `r.buffer` includes the distance
  limit; `r.neighbors -c` takes `dr² + dc² <= (size / 2)²`; null cells
  with a non-null neighbour get a sum.
  GDAL's XYZ export (`gdal_translate -of XYZ`) converts values to single
  precision: compare full-precision rasters, not XYZ dumps.
- Optimization: `exactextractr` returns each sum in single precision for a
  Float32 population (AM5 values are ours rounded to float32); its
  catchment argument is a file pattern in the shapes directory
  (`amGetShapesList`). Candidates covering no one are dropped only after a
  step that reduced some catchment (early returns), and touching counts
  as intersecting.

### Engine

- Outputs must be byte-identical whatever `--memory` and `--threads`; the
  tests enforce it. Any new output must keep that property.
- Tile size 64 is the default: 256² tiles thrash once the Dijkstra frontier
  band no longer fits (`benchmark/solver/README.md`).
- Referral `search_from: auto` (reversed searches) equals forward searches
  only up to floating-point summation order (~1e-15 relative); compare with
  a tolerance, or use `search_from: origins` for exact equality.
- Every budget split must count per-worker state (queue, state tiles) and
  results held in memory; Codex reviews caught these twice.
- GeoPackage accepts any attribute filter in `set_attribute_filter` and
  fails only when reading (no feature, an error on stderr). Validate a
  filter by running `SELECT * FROM "layer" WHERE …` with `execute_sql`
  (`vector::set_filter`), before setting it: releasing a generic OGR SQL
  result clears the filter of its source layer (FlatGeobuf).
- OGR SQL fails ("memory exhausted") on an `IN` list of thousands of
  values, except on GeoPackage (SQLite). Facility `ids` are applied while
  reading (`VectorLayer::select`, `VectorLayer::features`).

### Tooling

- Client: types come from `web/schemas` (`cargo xtask schemas`, then
  `web/scripts/gen-types.mjs` into the ignored `web/src/generated`); labels
  from `design_system/design-system/src/i18n/catalog.json` (`cargo xtask
  i18n`; never edit it, add keys to `own.json`). Build a screen from its
  AM5 reference (`design_system/references/inventory.md`) and the rules in
  `design_system/docs/DESIGN.md`, composing `@accessmod/ui/am/*`. The e2e
  test serves the built client on a fresh data directory (port 5198,
  `--no-local-paths`: files go through uploads) and writes screenshots,
  light and dark, to `web/test-results/screens/`.
- The design system is consumed from its sources: inside it, imports use
  the package-scoped `#ui/*` (`package.json` `imports`, one explicit entry
  per target: patterns do not probe extensions); never an `@/` alias.
- `sh scripts/check.sh` runs the whole Rust gate (fmt, clippy, tests,
  layers, LUT, schemas) and stops on the first failure. Never pipe a gate
  into `tail` before deciding to commit: the exit status is lost.

- The toolchain is pinned to Rust 1.92: `sysinfo` is held at 0.38 (0.39
  needs 1.95). Raising the toolchain is a user decision.
- `gdal` 0.19 is needed for GDAL 3.12 (0.18 has no matching bindings); it
  ships prebuilt bindings, no libclang required.
- Benchmarks live in `benchmark/<module>/` (`run.sh`, `am5.sh`, `README.md`)
  on the shared `benchmark/lib.sh`. `report.md` is generated by `run.sh`
  (machine, versions, command, median of `REPEAT` runs): never edit it by
  hand, and quote figures elsewhere only from a report. Run nothing else
  (no cargo build) while a benchmark runs.
- Benchmark scripts are POSIX `sh` (zsh has no `time -p`); on macOS,
  `/usr/bin/time -l` reports peak RSS, on Linux `/usr/bin/time -v`. Inside
  the AccessMod 5 image, time with `/proc/uptime` (its `date` has no `%N`),
  and never assign R globals such as `config` in a replay script. Give each
  container run its own output directory: a directory removed and made
  again under the same name may not be writable through the Docker
  bind mount (`am5_session` in `benchmark/lib.sh`).
- Run the checks and stop on failure before committing: a command chain
  that pipes clippy through `grep` does not stop on errors.
- Reviews by Codex: `codex exec -s read-only "<precise brief>"`, output kept
  in `_shared/reviews/`. Give it the commits, the contracts to check and
  where the oracle source is.

## Branches and releases

- `main` always builds and passes all checks.
- Short-lived feature branches merged into `main`.
- Releases are tag-driven (`vX.Y.Z`); versioning is independent from
  AccessMod 5.
- Releasing: review `git log <last tag>..main`, commit what changed in
  `docs/coverage.md` (and its "Last updated" line), the README status and
  AGENTS; then `npm run release -- X.Y.Z[-alpha.N]` (`--dry-run` first if
  unsure). It runs every gate, sets the version everywhere (Cargo, npm,
  R engine pin, README badge), tags and pushes; the tag publishes.
