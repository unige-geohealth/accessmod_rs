# accessmod-rs — general plan

Revision 2026-10-09, derived from `accessmod_app/plans/rs-refactor.md` after
review.

## Decision

The Rust engine becomes the **reference implementation** of AccessMod and will
eventually replace the whole AccessMod 5 stack (R/Shiny, GRASS, Docker), which
is neither maintainable nor scalable. Consequences:

- the AccessMod 5 plan `P1-01 r.accessmod` (unified GRASS engine) is dropped;
  no new engine is developed on the AccessMod 5 side;
- AccessMod 5 receives fixes only; it is a frozen **numerical oracle**
  (`5.9.2-beta.6`, see `reference-accessmod5.md`);
- this repository is standalone: no worktree, no submodule, no copied code.
  License `LGPL-3.0-or-later`; the public binary is `accessmod`.

## Target environments

| Profile | RAM | CPU | Expectation |
| --- | --- | --- | --- |
| Small machine / field laptop | 2 GB | 2 | National-scale runs complete, slower, within budget |
| Compute server | 40 GB | 12 | Same results, faster; large grids mostly cached in RAM |

Results must be **identical** on both profiles: memory budget and thread count
change speed, never numbers.

## Trajectory

1. Travel-time CLI engine on disk-backed tiled storage, without GRASS.
2. MCP stdio server and agent skill.
3. Migration of the other analyses.
4. Graphical UI once the engine and its contracts are stable.

## Architecture

### Workspace

- `accessmod-core`: tiled grid storage, mobility models, multi-source
  Dijkstra, domain types. No GDAL, no direct I/O beyond its own scratch
  storage, `forbid(unsafe_code)` unless a reviewed mmap layer requires it.
- `accessmod-io`: GDAL/PROJ, CRS, facility rasterisation, job validation,
  reports.
- `accessmod-cli`: `clap`, diagnostics, progress, exit codes.
- Milestone 2: `accessmod-mcp`, thin adapter over the same public API.
- `xtask`: developer tasks (LUT generation, oracle export driver).
- Rust 2024, pinned toolchain, committed `Cargo.lock`, `thiserror`, `serde`,
  `schemars`, `tracing`.

GDAL/PROJ stay at the I/O boundary ([GeoRust GDAL](https://github.com/georust/gdal)).
Internal geometries: `geo-types` / [`geo`](https://github.com/georust/geo).
No call to GRASS, R, Docker or any external GIS process.

**Tooling**: building, testing and generating artefacts require only the Rust
toolchain. Helper scripts are `cargo xtask` or POSIX `sh`. No R or Python in
the build or test path. Docker is used only to export the AccessMod 5 oracle.

### Units

- Internal values and canonical outputs: **seconds** and **metres**. Speeds
  are given in km/h in scenarios and converted once at the boundary.
- Minutes, kilometres and rounding are presentation only: reports, summaries,
  display, compatibility exports.

### CLI contract

```text
accessmod doctor --json
accessmod dataset inspect <path> --json
accessmod travel-time schema
accessmod travel-time validate --job <job.json> --json
accessmod travel-time run --job <job.json> [--json]
                          [--memory <size>] [--threads <n>] [--tmp-dir <path>]
```

- `--job` is mutually exclusive with direct parameters. Direct mode is part of
  v1: it accepts the same inputs as flags and always writes
  `job.resolved.json`.
- Human messages and progress go to stderr; stdout stays JSON.
- Exit codes: `0` success, `2` validation, `3` I/O, `4` computation,
  `130` cancelled.
- Output directory must be new or empty; no implicit overwrite in v1.
- `Ctrl-C` cancels cleanly and removes temporary artefacts, including scratch
  tiles.
- `--memory`, `--threads` and `--tmp-dir` are execution settings: recorded in
  `run.json`, never part of the scientific job.

### Travel-time job v1

Versioned JSON document:

- `schema_version: 1`;
- DEM, always required: it defines the grid (CRS, resolution, origin,
  extent), as in an AccessMod 5 project;
- land-cover raster;
- facility point layer and identifier field;
- scenario: class → `walking` | `bicycling` | `motorized` + speed > 0 km/h;
- method `isotropic` | `anisotropic`;
- direction `from_facilities` | `towards_facilities`;
- neighbourhood `8` | `16`;
- optional limit in seconds;
- named mobility model, initially `accessmod5-v1`;
- output directory.

The DEM is the absolute reference. Every other raster must lie on the DEM
lattice, otherwise validation fails; there is no resampling or reprojection
on the fly:

- CRS: semantic equivalence (GDAL/PROJ `IsSame`), not WKT string equality;
- resolution: relative difference ≤ 1e-9;
- origin: offset from the DEM origin within 1e-6 cell of a whole number of
  cells, on each axis;
- extent: any positive overlap with the DEM (it may be larger, e.g.
  population); the raster is windowed to the DEM extent and uncovered cells
  are NoData. Each raster keeps its own NoData value.

v1 rejects geographic CRS, rotated grids and non-metric units.

**Validation policy**: anything that makes the result wrong or undefined is a
blocking error (exit code `2`); anything else is a passive warning in the
report. Interactive fixes belong to a later TUI, not to the engine.

Facilities may be reprojected from another CRS. Points outside the grid, on a
barrier or on a class without scenario: detailed error by default, with an
explicit option to skip them with a warning (AccessMod 5 behaviour must be
characterised so oracle cases start from the same facilities). When several
facilities fall in one cell, the first one in the order of the facility layer
wins; the others are listed in a warning.

### Canonical outputs

Written atomically (temporary directory, then rename), streamed tile by tile
as tiled, compressed GeoTIFF:

- `travel_time_seconds.tif`, Float64;
- `nearest_facility.tif` (cost allocation), UInt32, with a JSON table
  ordinal → original facility identifier;
- `path_distance_m.tif`, Float64: planar (2D) length of the least-time path,
  sum of the step lengths L;
- `direction.tif`, UInt8, optional; codes 1–16 in AccessMod 5 neighbour
  order, 0 = facility cell, 255 = NoData (unreached) (`docs/cost-model.md`),
  used by Referral;
- `job.resolved.json`;
- `run.json`: versions, durations, parameters, execution settings, peak memory,
  scratch disk used, statistics, input fingerprints, warnings, known
  deviations from AccessMod 5. Fields are split into a deterministic part
  (job, inputs, statistics, warnings) and an execution part (durations,
  memory, threads, host); only the outputs and the deterministic part are
  covered by the byte-identical requirement.

Rounded minutes are a compatibility export only (`ceil` by default, as in
`amCleanTravelTime`). The canonical result stays in seconds.

## Storage and memory

Disk-backed tiled storage is a **milestone 1 requirement**, not a later
optimisation. RAM-only is too risky for the 2 GB target.

- **Tiled store**: every grid (inputs and solver state) is split into fixed
  square tiles (size tuned by benchmark, e.g. 256×256). Tiles live in scratch
  files under `--tmp-dir` and are cached in an LRU pool bounded by the memory
  budget. Same principle as the GRASS segment library used by `r.walk`.
- **Locality**: Dijkstra expands as a wavefront, so the working set is the band
  of tiles around the frontier; this is what makes an LRU tile cache
  effective. Tile access order is measured in benchmarks (hit rate in
  `run.json`).
- **Fast path**: when the whole working set fits in the budget, the same store
  keeps every tile resident and never touches disk. One code path, two
  regimes.
- **Solver state per cell**: time f64 (8) + distance f64 (8) + nearest u32
  (4) + predecessor u8 (1) + settled flag, plus inputs (class/speed, DEM).
  About 25–30 bytes per cell in total; 10⁹ cells ≈ 30 GB, which must run
  from disk on a 2 GB machine.
- **Priority queue**: kept in memory; its size grows with the frontier, not
  the grid. Its peak size is measured; a spilling or bucketed queue is added
  only if benchmarks show the frontier exceeds the budget. Status: the
  pre-flight reserves 10 % of the budget for it, but the queue is not bounded
  by that reserve (known gap; `heap_peak` is reported in `run.json`).
- **Budget**: `--memory` defaults to a conservative fraction of available RAM
  with a floor (values set by benchmark). The budget covers the tile cache, the
  queue and the GDAL block cache (`GDAL_CACHEMAX` set from the budget).
- **Pre-flight**: before computing, estimate memory and scratch disk (like
  AccessMod 5 `amGetRessourceEstimate`) and fail with exit code `2` and both
  estimates when the budget or free disk is insufficient.
- **Threads**: the v1 solver is single-threaded and deterministic. Threads
  (`--threads`, default = available CPUs, capped) are used for tile I/O,
  GDAL decode/encode and compression. Never oversubscribe a 2-CPU machine.

## Cost model

Specification of `accessmod5-v1`, derived from `mobility.h` and `main.c`
(exact references in `reference-accessmod5.md`):

- geometric step lengths: cardinal `res`, diagonal `√(ew²+ns²)`, knight
  `√(4ns²+ew²)` or `√(ns²+4ew²)`;
- **edge cost averages pace (1/v), not speed**: cardinal/diagonal
  `L · (1/v_a + 1/v_b) / 2`; knight `L · (Σ 1/v_i) / 4` over the start cell,
  end cell and the two crossed cells (one step along the long axis, on both
  columns or rows spanned by the move); v in km/h converted once, result in
  seconds;
- **one slope per edge**, `Δz / L` (ratio), applied to every cell of the edge;
  sign inverted for `towards_facilities`;
- isotropic: slope and mode ignored, scenario speed only. Oracle reference is
  `r.cost` on a seconds-per-cell friction, **not** `r.walk.accessmod`;
- anisotropic: walking (Tobler), motorized, bicycling (LUT);
- NoData (class or DEM), missing class, zero speed: barriers. A knight step
  crossing a barrier is forbidden, as in AccessMod 5. Deviation: AccessMod 5
  still assigns a motorized time to a cell with a null DEM (never expanded);
  here DEM NoData is a full barrier.

### Bicycle LUT

AccessMod 5 generates its LUT at build time with R
(`generate-bicycle-lut.R` → `bicycle-lut.generated.h` + `bicycle-lut.csv`) and
interpolates it bilinearly in `f64` over an `f32` table (flat speed × slope).
The production generator at `5.9.2-beta.6` is `generate-bicycle-lut.R`; the
`generate-bicycle-lut-new.R` variant is unused.

This repository becomes the **canonical source** of the model, without R:

- the model equations are implemented in Rust (`accessmod-core`, mobility
  module) and documented with their sources;
- `cargo xtask lut` evaluates them to produce the committed `f32` table
  embedded in the binary and an inspection CSV; CI checks the committed table
  is up to date;
- parity test: the Rust table is compared cell by cell with the `float`
  table compiled into the pinned `r.walk.accessmod` binary (extracted by
  `scripts/oracle/export.sh`; AccessMod 5 does not ship `bicycle-lut.csv`).
  Status: **bit-identical** on all 88 641 cells. Anchors and interpolation
  error (max 0.311 km/h, p99 0.032 km/h over 1–40 km/h) are checked;
- the model is frozen as `accessmod5-v1`; any change (e.g. reviewer feedback)
  creates a new version.

## Solver

- Deterministic multi-source Dijkstra; tie order: cost, physical distance,
  facility order, linear cell index.
- Produces travel time, nearest facility, path distance and optional
  predecessor in one pass.
- The cost limit is inclusive (`t <= limit`) and exact: no one-step overshoot
  as in `r.walk`, which AccessMod 5 removes afterwards in `amCleanTravelTime`
  with the same inclusive rule.

## Milestones

### Milestone 0 — oracle

- Committed script (POSIX `sh` driving the pinned AccessMod 5 Docker image)
  exporting into `_oracle/`: GeoTIFF/GeoPackage fixtures, scenarios, LUT CSV,
  **raw travel times in seconds before `amCleanTravelTime`**, nearest, for
  isotropic and anisotropic, motorized, walking and bicycling, 8/16
  neighbours, both directions, with and without limit.
- Committed manifest (files, SHA-256, oracle version).
- Characterisation needed before setting tolerances: cell type of the
  isotropic friction (`r.recode`), facilities on barriers. Behaviour settled
  from the source is listed in `reference-accessmod5.md`.
- Status (2026-10-09): `scripts/oracle/export.sh` exports the bicycle LUT
  and 8 travel-time cases on the AccessMod 5 demo project (227 × 196 cells,
  10 facilities, all three modes, 8/16 neighbours, both directions, limit).
  Results: LUT bit-identical; anisotropic relative error ≤ 4e-16; isotropic
  ≤ 3.7e-8 (friction stored as Float32); reachable masks and nearest
  facility identical on every cell. Not yet characterised: facilities on
  barriers.

### Milestone 1 — travel-time CLI engine

- Workspace, tiled disk-backed store with memory budget and pre-flight.
- Cost model and LUT above; deterministic solver.
- Streaming GeoTIFF outputs.
- `doctor`: GDAL/PROJ versions, GeoTIFF/vector drivers, PROJ data, scratch
  directory writable, free disk and RAM.
- Distribution: evaluate first the bundled GDAL/PROJ build (`gdal-sys`
  `bundled` feature / `gdal-src`, or a minimal GeoTIFF + GPKG build) on the
  five targets; this is the main risk of the milestone.

### Milestone 2 — MCP and skill

- stdio server with [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk).
- Tools: `inspect_dataset`, `get_travel_time_schema`,
  `validate_travel_time_job`, `run_travel_time`, `read_run_report`.
- MCP long-running tasks for progress and cancellation.
- Paths restricted to client roots; no network; no overwrite.
- `skills/accessmod/SKILL.md`: inspect → build/validate job → run → check
  `run.json`, without copying the schema; validated on a realistic scenario.

### Later

1. Zonal statistics, done (2026-10-16) ([`zonal-statistics.md`](zonal-statistics.md));
   geographic coverage and catchments, done (2026-10-16)
   ([`geographic-coverage.md`](geographic-coverage.md)).
2. **Referral**, done (2026-10-09) except the network output: many-to-many with early
   stop, smaller-side searches and parallel sources ([`referral.md`](referral.md)).
3. Scaling up, done (2026-10-16) ([`scaling-up.md`](scaling-up.md)); optimization,
   done (2026-10-16) ([`optimization.md`](optimization.md)).
4. New project format; `.am5p` import as a separate converter.
5. UI on top of the stabilised CLI/MCP APIs: web front end served by
   `accessmod serve`, optional Tauri package, no TUI ([`ui.md`](ui.md)).
6. Land-cover merge (land cover, roads with classes, barriers, bridge
   cleaning), done (2026-10-16) except the polygon skeleton option
   ([`merge-land-cover.md`](merge-land-cover.md)); population adjustment on
   barriers, done (2026-10-16) ([`adjust-population.md`](adjust-population.md)).
   Both are prerequisites of coverage analyses on raw data. See
   [`coverage.md`](coverage.md).

## Tests and acceptance criteria

- Unit: cardinal/diagonal/knight costs, pace averaging, slope inversion, three
  modes, LUT, limits, barriers, NoData, ties.
- Property: non-negative costs, monotonic distance along paths, flat speed
  preserved, determinism independent of facility order.
- **Storage invariance**: the same job run with a tiny budget (forcing
  eviction to disk), a large budget and different thread counts produces
  byte-identical outputs (execution part of `run.json` excepted).
- Hand-computable synthetic fixtures (committed).
- Oracle comparison:
  - identical geometry and reachable mask;
  - **identical graph (anisotropic, f64)**: relative error ≤ 1e-9;
  - **isotropic**: relative error ≤ 2⁻²⁴: `r.recode` stores the friction as
    FCELL (Float32), so each friction and every sum of frictions carries a
    relative rounding error up to 2⁻²⁴;
  - `nearest` identical except on mathematically tied cells;
  - any other deviation (DEM NoData for motorized, limit overshoot, tie-break)
    documented in `docs/` and reported in `run.json`.
- Integration: isotropic/anisotropic, 8/16 neighbours, multi-source, limit,
  both directions.
- Resource tests: a national-scale grid completes under a 2 GB / 2 CPU
  container limit; peak RSS stays within the configured budget.
- Versioned benchmarks on both target profiles; no performance promise before
  the first baseline.
- CI: Linux x86_64/arm64, macOS x86_64/arm64, Windows x86_64.
- Self-contained archives (`accessmod`, GDAL/PROJ, data, third-party
  licenses); no promise of a single static executable. Each archive runs
  `doctor` and a small GeoTIFF computation as a smoke test.

## Assumptions

- v1 reads neither GRASS databases nor `.am5p` archives.
- GeoTIFF is the canonical raster format; any GDAL-readable point layer is
  accepted.
- Clarity, determinism and auditability first; solver parallelism only after
  a correct baseline.
- AccessMod 5 behaviour is reproduced when correct; proven defects are fixed
  and documented, not reproduced.

## License

- `LGPL-3.0-or-later` (`LICENSE`, with the GPL-3.0 text in `COPYING`),
  consistent with AccessMod 5.
- `r.walk.accessmod` derives from GRASS (GPL ≥ 2): the algorithm is
  reimplemented from its specification, the code is not translated.
- Check license compatibility of every dependency and bundled library
  (GDAL: MIT; PROJ: MIT).
