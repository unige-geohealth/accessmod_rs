# R package

Status (2026-11-18, 6.0.0-alpha.1): **experimental**, in [`r/`](../r/README.md).
Installable from GitHub on Windows, macOS and Linux (x86_64, arm64); not on
CRAN.

## Goal

Let R users evaluate many scenarios on a project built in the web interface:
change speeds, roads or barriers, regenerate the merged land cover, compute
travel times, compare. Interactive work stays in the web interface; R
drives batches and analysis of the results.

## Design: the binary and a thin R layer

The package contains only R code. It runs the AccessMod 6 command line,
which is already the reproducible interface of every analysis (job
documents, JSON output, exit codes; [`plan.md`](plan.md)):

- `am_install()` downloads the release archive of the platform (the same as
  the [distribution](distribution.md): GDAL, PROJ and the web client inside)
  into `tools::R_user_dir("accessmod", "data")`, after checking its SHA-256.
- Every function builds the arguments, runs the binary (`system2`), and
  parses its JSON output; a failure stops with the command's own issues.
- Objects are S3 values: `am_project` (a directory and its metadata),
  `am_job` (analysis and job document, copied on change, so scenario loops
  never alias), `am_run` (the run record). State lives in the project, not
  in R.

Why not a compiled package (extendr, a Rust crate built by R): R would
build GDAL, PROJ and GEOS from source on each machine (Rtools on Windows,
minutes of compilation, CRAN's build rules), for no gain: an analysis runs
seconds to hours, the process start is milliseconds. The binary is the one
the web interface and the CLI use, tested in CI on the five targets. CRAN
refuses packages that download binaries; GitHub (`remotes`, `pak`) and
later r-universe do not.

## In alpha.1

| Need | Functions |
| --- | --- |
| Binary | `am_install()`, `am_binary()` (`ACCESSMOD_BIN`, option `accessmod.binary`), `am_version()`, `am_doctor()` |
| Projects | `am_project_import()` (`.am6p`, AccessMod 5 data archive), `am_project()`, `am_project_demo()`, `am_datasets()`, `am_runs()` |
| Jobs | `am_jobs()`, `am_job()` (saved in the web interface), `am_job_new()`, `am_job_read()`, `am_scenario()`, `am_scenario<-` |
| Runs | `am_run()` (any analysis), `am_merge_land_cover()`, `am_travel_time(…, land_cover = merge_run)` |
| Results | `am_outputs()` (datasets with mean, min, max), `am_raster()` (`terra`) |

Command line additions for it: `accessmod project import` (a project file or
an AccessMod 5 archive) and `accessmod project jobs` (saved jobs, JSON).

Tests: unit tests always; `tests/testthat/test-demo.R` runs the demo end to
end (merge, two travel-time scenarios, slower roads give longer times) when
a binary is found. CI (`.github/workflows/r.yml`): `R CMD check` on Windows,
macOS, Linux x86_64 and arm64 on every change of `r/`; after a release, the
same with `am_install()` from that release and the end-to-end test.

## Planned

- Functions for each analysis with its main options (referral, zonal
  statistics, geographic coverage, scaling up, optimization), returning
  tables as data frames.
- Inputs from R objects: `sf` layers and `terra` rasters imported into the
  project (`accessmod dataset import`), e.g. a flood extent as a barrier.
- Project from a region (the Project Creator) from R, once it leaves
  experimental.
- Progress and cancellation (the CLI's progress on stderr; `processx`).
- Export of the project (`.am6p`) and of results.
- r-universe builds; signed macOS binary (notarization) so no Gatekeeper
  check is needed.

## Limits

- Linux archives are built on Ubuntu 24.04 and linked against its glibc:
  older distributions may lack the glibc version they need (not measured
  yet). There: build from source, or use a container.
- Windows on arm runs the x86_64 build (emulation).
- Tags of a run must be new for its output classes, unless
  `overwrite = TRUE`: give each scenario its own tags.
