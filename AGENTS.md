# AGENTS.md — accessmod-rs

Guide for humans and coding agents (Claude Code, Codex, …) working in this
repository. Read [`docs/plan.md`](docs/plan.md) first: it is the source of
truth for architecture, contracts and milestones. Code style is in
[`STYLE.md`](STYLE.md).

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
- **Tooling dependencies**: building, testing and generating artefacts (e.g.
  the bicycle LUT) need only the Rust toolchain. Helper scripts are Rust
  (`cargo xtask …`) or POSIX `sh`. No R; no Python in the build or test path.
  Docker is needed only to export the AccessMod 5 oracle.

## Commits

- Conventional commits: `feat:`, `fix:`, `refactor:`, `perf:`, `test:`,
  `docs:`, `chore:`, `ci:`; optional scope, e.g. `feat(core): …`.
- **One line only. No body. No `Co-Authored-By` or any other trailer.**
- Commit only what the change needs; never commit `_oracle/`, `_am5/`,
  `_shared/` or `target/`.

## Multiple agents

Claude Code and Codex may implement the same task in parallel; the best
implementation is kept.

- Work in a dedicated branch per agent and task, e.g. `claude/<topic>`,
  `codex/<topic>`, branched from `main`.
- Keep each branch self-contained and passing all checks so it can be
  compared and merged as a unit.
- Do not edit another agent's branch.
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
- **Vocabulary**: domain names (CLI, job fields, outputs, types, docs) follow
  the English names in `www/dictionary/classes.json` and `main.json`, e.g.
  *facility* (French *structure*), *land cover*, *cost allocation*. "Source"
  is reserved for solver internals.

## Commands

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo xtask lut [--check]  # regenerate (or verify) the bicycle LUT
sh scripts/oracle/export.sh # export AccessMod 5 oracle artefacts into _oracle/ (Docker)
cargo run -p accessmod-cli -- doctor --json
cargo run -p accessmod-cli -- travel-time run --job job.json --json
cargo run -p accessmod-cli -- referral run --job referral.json --json
sh scripts/bench/am5-vs-rs.sh 1000 2000      # travel-time benchmark vs AccessMod 5 (Docker)
sh scripts/bench/referral-vs-am5.sh 1000 20  # referral benchmark vs AccessMod 5 (Docker)
```

The workspace needs GDAL ≥ 3.5 and PROJ installed (`gdal-config` on `PATH`).

Build, test, clippy and fmt must all pass before a commit.

## Layout

```
crates/accessmod-core   tiled storage, raster primitives, mobility and cost models, solver, referral, merge, population, zonal (no GDAL)
crates/accessmod-io     GDAL/PROJ boundary: GIS toolkit, jobs, analyses, outputs, reports; fixtures (tests); examples/
crates/accessmod-cli    `accessmod` binary (clap): doctor, dataset, travel-time, referral, merge-land-cover, adjust-population-distribution, zonal-statistics
crates/accessmod-mcp    milestone 2 (not yet): thin MCP adapter over the public API
xtask/                  developer tasks (`cargo xtask lut`)
_oracle/                AccessMod 5 exports (ignored; checksummed manifest committed)
_am5/accessmod/         pinned AccessMod 5 source, read-only (ignored; `scripts/fetch-am5.sh`)
_shared/                local scratch, issue reproductions (ignored)
scripts/                POSIX sh helpers: `oracle/` (export + committed manifest), `bench/`
docs/                   plan, references, design decisions, tutorial (with data and figures)
```

`accessmod-core` never depends on GDAL, PROJ, a project layout or an external
process.

## Reusable GIS toolkit

Analyses are short compositions of shared, low-level GIS building blocks,
the way AccessMod 5 composes GRASS modules (`v.to.rast`, `r.patch`,
`r.mapcalc`). An analysis module holds only its domain rules; the
mechanics live in the toolkit and are written, tested and reviewed once:
determinism, memory budget, cancellation and error messages included.

| Need | Toolkit |
| --- | --- |
| Rasterise lines and polygons, 3 × 3 neighbourhoods | `accessmod_core::raster` |
| Disk-backed grids under a memory budget | `accessmod_core::store` |
| Open a class raster, read any raster aligned on the DEM | `accessmod_io::raster` |
| Open a vector layer (filter, CRS, reprojection), burn features | `accessmod_io::vector` |
| DEM grid, lattice alignment, map ↔ pixel coordinates | `accessmod_io::grid` |
| Job files, path resolution, schema version, input checks | `accessmod_io::job` |
| Memory plan and execution settings | `accessmod_io::resources` |
| Staging directory, GeoTIFF blocks, CSV, `finish` | `accessmod_io::output` |
| `run.json` (`Report`, `Execution`), fingerprints, warnings | `accessmod_io::report` |
| `prepare` / `run` contract driven by the CLI (and MCP) | `accessmod_io::analysis::Analysis` |
| Test inputs, oracle paths, running a job | `accessmod_io::fixtures` (feature `fixtures`), `accessmod-core/tests/common` |

Rules:

- No copy of a helper in an analysis, a command or a test. Before writing
  one, look in the toolkit; if it is not domain-specific, add it there with
  its own test and use it from every caller.
- A new analysis implements `Analysis`; its CLI command is
  `exec::job_command::<A>` unless it needs direct parameters.
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

### Engine

- Outputs must be byte-identical whatever `--memory` and `--threads`; the
  tests enforce it. Any new output must keep that property.
- Tile size 64 is the default: 256² tiles thrash once the Dijkstra frontier
  band no longer fits (`docs/benchmarks.md`).
- Referral `search_from: auto` (reversed searches) equals forward searches
  only up to floating-point summation order (~1e-15 relative); compare with
  a tolerance, or use `search_from: origins` for exact equality.
- Every budget split must count per-worker state (queue, state tiles) and
  results held in memory; Codex reviews caught these twice.
- GeoPackage accepts any attribute filter in `set_attribute_filter` and
  fails only when reading (no feature, an error on stderr). Validate a
  filter by running `SELECT * FROM "layer" WHERE …` with `execute_sql`.

### Tooling

- The toolchain is pinned to Rust 1.92: `sysinfo` is held at 0.38 (0.39
  needs 1.95). Raising the toolchain is a user decision.
- `gdal` 0.19 is needed for GDAL 3.12 (0.18 has no matching bindings); it
  ships prebuilt bindings, no libclang required.
- Benchmark scripts are POSIX `sh` (zsh has no `time -p`); on macOS,
  `/usr/bin/time -l` reports peak RSS.
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
