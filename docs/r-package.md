# R package

Status (2026-11-18): **experimental**, in [`r/`](../r/README.md); second
design (R6 sessions), after alpha.1's first use. Installable from GitHub on
Windows, macOS and Linux (x86_64, arm64); not on CRAN.

## Goal

Let R users evaluate many scenarios on a project built in the web interface:
change speeds, roads or barriers, regenerate the merged land cover, compute
travel times, compare. Interactive work stays in the web interface; R
drives batches and the analysis of the results.

## Design

The package contains only R code and runs the AccessMod engine (the
`accessmod` command line, the reproducible interface of every analysis: job
documents, JSON output, exit codes; [`plan.md`](plan.md)).

**Objects** (R6, every one read only; decided with a Codex review):

| Object | Made by | Holds |
| --- | --- | --- |
| `AccessMod` (session) | `am_init(path, import, demo, binary)` | the project path and the engine; every method reads the project, so sessions and the web interface can share it |
| `Job` | `am$get("job", …)`, `am$job()`, `am$new_scenario()`, `job$with()` | analysis and job document, private; `clone()` disabled; changes return new jobs |
| `Dataset` | `am$get(type, …)`, `am$import_data()`, `run$get_output()` | the catalogue row; `path()`, `read()` |
| `Run` | `am$run()` | the run record; `list_outputs()`, `get_output()` |

**Rules** (keep the API from drifting):

1. **It matches the Data screen.** The catalogue is `dataset list` plus
   `project runs`. A job is the job document of a finished analysis run,
   with the run's id and tags, as Data › Job. Outputs are found by the
   dataset ids a run recorded; missing ones (a project file without
   results) are reported as such.
2. **Selection is predictable.** `type` ∈ raster, vector, table, job;
   `class` is the class code, or the analysis for jobs; every given tag
   must be among the row's tags, in any order. `list_*` may return no row;
   `get` wants exactly one and lists the candidates otherwise.
3. **Jobs are immutable.** Private state of JSON values, no `clone()`;
   `with()` and `new_scenario()` build new jobs. `new_scenario(job,
   speeds)` changes only the classes given, each once, finite and ≥ 0, all
   in the job's scenario. Tested: a baseline is unchanged, aliases
   included, after deriving variants.
4. **The engine owns the contract.** `am$job()` and `with()` take the job
   schema's fields; a Dataset becomes its project-relative path (for an
   object field such as `facilities`, its `path`); a Dataset or Job of
   another project is refused. One runner for every analysis; the engine
   validates and chooses the output directory.
5. **The engine is pinned and the boundary is tested.** The package names
   the engine version it is made for (`Config/accessmod/engine`);
   `am_init()` refuses another major version and warns on another one.
   Failures are `accessmod_error` conditions with the engine's kind, exit
   status and issues. Tests: arrays of one element, ambiguous selections,
   alias safety, checksum refusal, and the demo end to end.

**Engine distribution.** The engine (about 20 MB, GDAL and PROJ inside) is
the release archive of the [distribution](distribution.md), installed by
`am_install()`: the pinned version, its `.sha256` required, unpacked aside,
`doctor` run, then activated; `file =` for offline installs;
`am_uninstall()`. Why not compile it in the R package (extendr,
r-universe): R would build Rust, GDAL, PROJ and GEOS on every machine
(Rtools, cmake, minutes), a second build pipeline for the same engine.
R-universe may later distribute the R code only. The desktop
application's engine is used only when named (`binary`, `ACCESSMOD_BIN`),
as it needs `PROJ_DATA` from the application's resources.

Command line additions for the package: `project import` (a project file
or an AccessMod 5 archive), `project export`, `project jobs`.

**CI** (`.github/workflows/r.yml`): `R CMD check` on Windows, macOS, Linux
x86_64 and arm64; on Linux also with an engine built from the commit (the
end-to-end test then runs); after a release, `am_install()` of that release
and the end-to-end test on the four platforms.

## Planned

- Tables as data frames for the analyses that write them (zonal
  statistics, referral, coverage), and helpers to compare runs.
- Inputs from R objects: `sf` or `terra` layers written to a file and
  imported (`am$import_data()` takes files today).
- Project from a region (the Project Creator) once it leaves experimental.
- Progress and cancellation of long runs.

## Limits

- Linux archives are built on Ubuntu 24.04 and linked against its glibc:
  older distributions may lack the glibc version they need (not measured
  yet). There: build from source, or use a container.
- Windows on arm runs the x86_64 build (emulation).
- The tags of a run must be new for its output classes, unless
  `overwrite = TRUE`: give each scenario its own tags.
