# Coverage of AccessMod 5 features

What AccessMod 6 covers today, in the engine (command line, job files) and
in the web interface (`accessmod serve`), and what is planned. Reference:
AccessMod 5 `5.9.2-beta.6` (modules and `tools/R` paths are relative to
`_am5/accessmod/`). Update this file with every feature change; the README
points here for the project status.

Status: ✅ available · 🟡 partial · 🗓 planned (milestone in
[`plan.md`](plan.md)) · ➖ not planned as such (replaced or out of scope).
**Engine**: command line and job files, checked against AccessMod 5 (the
oracle). **Web**: the screen in the web interface.

_Last updated: 2026-10-31 (6.0.0-alpha.1)._

## Analyses

| AccessMod 5 feature | Where in AM5 | Engine | Web |
| --- | --- | --- | --- |
| **Accessibility**: travel time to/from facilities | `amAnalysisTravelTime.R` | ✅ `travel-time run`; matches AM5 to 1e-15 (anisotropic) and 2⁻²⁴ (isotropic) on the demo project | ✅ Analysis › Accessibility |
| Isotropic / anisotropic, from / towards facilities, knight's move | idem | ✅ | ✅ |
| Walking (Tobler), bicycling (LUT), motorized | `mobility.h` | ✅ bicycle LUT bit-identical | ✅ scenario table, editable |
| Maximum travel time | idem | ✅ `limit_s` (seconds) | ✅ minutes |
| Nearest facility (cost allocation) | idem | ✅ `nearest_facility.tif` + ID table | ✅ outputs |
| Output in minutes, rounding, min travel time, timeout value | `amCleanTravelTime` | 🗓 compatibility export; canonical output stays in seconds | 🗓 |
| Path distance, movement direction | `r.walk` `outdir` | ✅ `path_distance_m.tif`, `direction.tif` | ✅ outputs |
| Facility selection (subset of a layer) | `amFacilitiesSubset`, facility table | ✅ `where` attribute filter and explicit `ids` in the job (a subset replays headless) | ✅ table with checks, filter, AM5 check columns |
| Facilities on barriers: detected, user deselects | facility validation | ✅ listed as errors, or skipped with `facility_issues: skip` | ✅ `amOnBarrier`, `amOnZero` columns |
| Region cropping by max speed × time | `amSpeedBufferRegion.R` | ➖ not needed: the limit is applied exactly during the search | — |
| **Geographic coverage**: capacity, catchments, residual population | `amAnalysisCapacity.R`, `amAnalysisCatchment.R` | ✅ `geographic-coverage run`: equal to the AM5 demo replays ([`geographic-coverage.md`](geographic-coverage.md)); 🗓 circular-buffer order | ✅ Analysis › Geographic coverage |
| **Zonal statistics**: population coverage by zone and time | `amAnalysisZonal.R` | ✅ `zonal-statistics run`: equal to AM5 on the demo ([`zonal-statistics.md`](zonal-statistics.md)) | ✅ table in place, columns per time step, CSV/JSON |
| **Referral**: time and distance between facilities | `amAnalysisReferralParallel.R`, `amAnalysisTimeDist.R` | ✅ `referral run`: times equal AM5 (after its rounding), distances within one cell ([`referral.md`](referral.md)); 🗓 network lines output | ✅ Analysis › Referral (From / To) |
| **Scaling up**: suitability, exclusion, new facility placement | `amAnalysisScalingUp.R` | ✅ `scaling-up run`: equal to the AM5 demo replay ([`scaling-up.md`](scaling-up.md)) | ✅ capacity, suitability and exclusion tables |
| **Optimization**: greedy best population coverage | `amAnalysisOptimization.R` | ✅ `optimization run`: equal to the AM5 demo tests ([`optimization.md`](optimization.md)) | ✅ Analysis › Optimization |
| Replay of saved analyses | `amAnalysisReplay.R` | ✅ `job.json` / `job.resolved.json` replay any run; 🗓 converter from AM5 analysis configs | 🟡 Export job; 🗓 Import job |

## Toolbox

| AccessMod 5 feature | Where in AM5 | Engine | Web |
| --- | --- | --- | --- |
| **Merge land cover**: land cover, roads and barriers in priority order | `modules/amAnalysisMergeLandCover`, `amStackBarrier.R` | ✅ `merge-land-cover run`: 2 cells of 44 492 differ from AM5 on the demo ([`merge-land-cover.md`](merge-land-cover.md)); 🗓 polygon skeleton option | ✅ Toolbox › Merge land cover: land cover with editable labels, road classes, barriers (lines or polygons as the layer holds them, no type to choose), stack order, conflicts; 🗓 quick correction of conflicts |
| Bridge and artefact cleaning when merging | `amBridgeTools.R` | ✅ `clean_bridges`, `bridges.tif` identical to AM5 on the demo | 🗓 |
| Conflicting classes report | merge module | ✅ validation error naming the items; `reclass` (AM5 quick correction) | 🗓 |
| **Adjust population distribution** | `modules/amPopulationBarrierCorrection` | ✅ `adjust-population-distribution run`: equal to AM5 within 6e-15 ([`adjust-population.md`](adjust-population.md)) | ✅ Toolbox › Adjust population distribution |
| **Interactive map**: preview layers, relocate facilities | `modules/amGisPreview` | ✅ tiles, values at a point, features sampled on a raster, `dataset.relocate` | ✅ Toolbox › Interactive map, a GIS module: Facilities relocator (red when the reference raster has no value at the facility, Next, drag with undo and redo, saved as a new facilities layer), up to 10 additional vector layers in a z-order, basemaps (offline schematic; MapTiler backdrop, dataviz, satellite, outdoor with a key); ➖ updating the layer in place (runs reference it), automatic snapping (which side of a barrier takes context) |
| Demo project (AM5 sample data) | image data | ✅ `project demo` | ✅ Toolbox › Demo project |

## Project and data

| AccessMod 5 feature | Where in AM5 | Engine | Web |
| --- | --- | --- | --- |
| Project built from the DEM | `amProjectCreateFromDem` | ✅ `project init`: the DEM defines the grid; imports must lie on its lattice (stricter than AM5, which resamples); 🗓 explicit resampling | ✅ Project › New / Import, summary, location map, delete |
| Raster import with CRS / extent checks | `amUploadRaster.R` | ✅ GeoTIFF, ERDAS Imagine; no implicit resampling or reprojection | ✅ Data › Import (uploads) |
| Vector import (facilities, roads, barriers, zones) | `amUploadVector.R` | ✅ shapefile, GeoPackage, FlatGeobuf, GeoJSON; explicit reprojection | ✅ |
| Table import (scenarios, capacities, suitability, exclusion) | `amUploadTable.R` | ✅ CSV and XLSX | ✅ and editable in the analyses |
| Data archives: create from the selection, list, export, delete | `amManageData` (archive panel), `amExport.R` | ✅ `archive.create|remove`, `GET …/archives[/{name}]`: a zip keeping the project paths, each chosen run's job at its root (`job_run<id>.json`, output to `replay_run<id>`), `manifest.json` with SHA-256 per file, `README.txt`; native formats (no Shapefile/.img conversion), no overviews; a run whose inputs are in the archive replays to byte-identical outputs (tested) | ✅ Data › Archive (warns when a selected job misses inputs) |
| Data catalogue: tags, filter, rename, delete | `modules/amManageData` | ✅ `dataset import|list|tag|remove`; run outputs registered as datasets, run inputs recorded (`RunRecord.inputs`) | ✅ Data (filter, last results, rename, delete); the job of each finished run is a row (type Job, in place of AM5 Config); a run filter (Jobs › Show in Data, Filter last results) lists its job, inputs and outputs, selected |
| Project export / import (`.am5p`) | `amProjectImportExport.R` | 🗓 converter | 🗓 |
| Data export | `amExport.R` | ✅ GeoTIFF, FlatGeobuf, CSV, JSON | 🟡 tables as CSV/JSON |
| Logs | `modules/amManageLogs` | ✅ `run.json` per run, `GET /api/runs` | ✅ Jobs, in place of Logs: runs of every project, progress, cancel, details, outputs, job export |
| Settings, languages (en, fr, es, pt) | `modules/amManageSettings`, `www/dictionary` | ➖ CLI flags | ✅ Settings: theme (follows the system, or light, or dark), four languages, version; ➖ AM5 admin tools (restart, cache, upload limit) |
| Resource checks before running | `amGetRessourceEstimate` | ✅ pre-flight memory and disk estimate, `--memory` budget | ✅ validation before Compute |

## Platform

| AccessMod 5 | AccessMod 6 |
| --- | --- |
| R/Shiny + GRASS GIS + Docker, packaged as Electron app or VM | One `accessmod` binary: command line and `accessmod serve` (web interface on the desktop); ✅ self-contained archives with GDAL/PROJ for Linux, macOS and Windows ([`distribution.md`](distribution.md)); 🗓 country server with accounts; 🟡 desktop application (Tauri window over the same engine, unsigned installers: [`desktop.md`](desktop.md)) |
| Interactive use only (plus replay) | Scriptable: job files, JSON on stdout, exit codes; every web action is a reproducible job; ➖ MCP server deferred |
| Memory needs grow with the grid (GRASS segments) | Explicit memory budget, tiled disk storage, same results at any budget ([benchmarks](../benchmark/README.md)) |
| Results in minutes (rounded) | Results in seconds and metres; minutes in the interface, compatibility export 🗓 |

## Next in the interface

Import job, `.am5p` import, Project › Export, the
quick correction of merge conflicts. More GIS tools in the interactive map,
e.g. a flood calculator (DEM and settings to barrier polygons for Merge
land cover, or a friction layer slowing travel in every analysis): to be
researched. Architecture, state rules and the screen-to-job mapping are in
[`ui.md`](ui.md).
