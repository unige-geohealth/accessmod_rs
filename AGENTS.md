# AGENTS.md — accessmod-rs

Guide for humans and coding agents (Claude Code, Codex, …) working in this
repository. Read [`docs/plan.md`](docs/plan.md) first: it is the source of
truth for architecture, contracts and milestones. The interface (project
layer, `accessmod serve`, `web/`) follows [`docs/ui.md`](docs/ui.md). Code
style, Rust and TypeScript, is in [`STYLE.md`](STYLE.md).

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
  toolchain; `cargo build --workspace` never runs Node. The front end in
  `web/` needs pinned Node LTS and npm, nothing else. Helper scripts are Rust
  (`cargo xtask …`) or POSIX `sh`. No R; no Python in the build or test path.
  Docker is needed only to export the AccessMod 5 oracle.
- **Interface state**: no cascades. Typed messages, pure `update`, derived
  values as selectors, no effect that sets state from other state
  ([`docs/ui.md`](docs/ui.md), "State"). Reviewers treat a violation as a
  defect.
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
sh scripts/oracle/export.sh # export AccessMod 5 oracle artefacts into _oracle/ (Docker)
cargo run -p accessmod-cli -- doctor --json
cargo run -p accessmod-cli -- travel-time run --job job.json --json
cargo run -p accessmod-cli -- referral run --job referral.json --json
cargo run -p accessmod-cli -- project init p --dem dem.tif --name Demo
cargo run -p accessmod-cli -- dataset import lc.tif --class rLandCover --tags base --project p
cargo run -p accessmod-cli -- project run travel-time --job job.json --project p   # paths relative to p
sh benchmark/travel-time/run.sh 1000 2000   # benchmark vs AccessMod 5 (Docker), writes report.md
sh benchmark/referral/run.sh 1000 20         # idem; also benchmark/geographic-coverage/, scaling-up/, optimization/
```

Interface (milestone 3):

```sh
cargo xtask schemas [--check]               # JSON Schemas into web/schemas/ (committed)
cargo xtask layers                          # crate dependency rules (core: no GDAL, …)
cargo run -p accessmod-cli -- serve --web-dir web/dist  # API + built front end, http://127.0.0.1:5099
npm --prefix web ci
npm --prefix web run dev                    # Vite dev server, proxies /api to `serve`
npm --prefix web run check                  # tsc --noEmit, Biome, Vitest
npm --prefix web run build && npm --prefix web run e2e  # Playwright: serve + tutorial data, end to end
```

The workspace needs GDAL ≥ 3.5 and PROJ installed (`gdal-config` on `PATH`).

Build, test, clippy and fmt must all pass before a commit; when `web/` or
the schemas change, `cargo xtask schemas --check` and `npm --prefix web run
check` too; once it exists, `cargo xtask layers`.

## Layout

```
crates/accessmod-core   tiled storage, raster primitives, mobility and cost models, solver, referral, merge, population, zonal, capacity, scaling up, optimization, focal operations (no GDAL)
crates/accessmod-io     GDAL/PROJ boundary: GIS toolkit, jobs, analyses, outputs, reports; fixtures (tests); examples/
crates/accessmod-cli    `accessmod` binary (clap): doctor, project, dataset, travel-time, referral, merge-land-cover, adjust-population-distribution, zonal-statistics, geographic-coverage, scaling-up, optimization
crates/accessmod-server milestone 3 (not yet): `accessmod serve`, command registry, JSON API + events generic over `Analysis`, accounts and roles, run queue, tiles, embedded front end (feature `web`)
web/                    milestone 3 (not yet): TypeScript front end (React, MapLibre, Tabulator); schemas/ generated by `cargo xtask schemas`
xtask/                  developer tasks (`cargo xtask lut`, `cargo xtask schemas`)
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
| Open a vector layer (filter, CRS, reprojection), burn features (dense lines, or `Burn::Thin` as `v.to.rast` without `-d`) and zones, polygonise cells, sums over polygons weighted by coverage, geometries spooled to disk, write FlatGeobuf | `accessmod_io::vector` (`burn_layer`, `burn_zones`, `polygonize`, `covered_sum`, `GeometrySpool`, `FeatureWriter`) |
| Residual population, band search, catchment polygon, removal (capacity analyses) | `accessmod_io::catchment` |
| DEM grid (or the grid of another raster, `open_raster`), lattice alignment, map ↔ pixel coordinates | `accessmod_io::grid` |
| Job files, path resolution, schema version, input checks | `accessmod_io::job` |
| Memory plan and execution settings | `accessmod_io::resources` |
| Staging directory, GeoTIFF blocks, CSV, `finish` | `accessmod_io::output` |
| Raster overviews in an external `.ovr`, statistics (range, histogram, classes) | `accessmod_io::preview` |
| Map previews: raster tiles in Web Mercator (PNG, through the overviews), GeoJSON in WGS 84, values at a point, bounds | `accessmod_io::tiles` |
| RFC 4180 CSV reading (quoted fields, bounded records) | `accessmod_io::output::read_csv` |
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
  filter by running `SELECT * FROM "layer" WHERE …` with `execute_sql`.

### Tooling

- Web: types come from `web/schemas` (`cargo xtask schemas`, then
  `npm run gen:types`, run by `dev`, `build` and `check`); never edit
  `web/src/generated`. The e2e test starts `accessmod serve` itself on a
  fresh data directory (port 5198) and needs `npm run build` first; its
  screenshots land in `web/test-results/`.
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
