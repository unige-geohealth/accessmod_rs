# Coverage of AccessMod 5 features

What accessmod-rs covers today, what is planned, and what needs a graphical
interface. Reference: AccessMod 5 `5.9.2-beta.6` (modules and `tools/R`
paths are relative to `_am5/accessmod/`). Update this file with every
feature change.

Status: ✅ available · 🟡 partial · 🗓 planned (milestone in
[`plan.md`](plan.md)) · ➖ not planned as such (replaced or out of scope).
Interface: **CLI** works from the command line or a job file; **UI** needs
an interactive interface to be practical (map, drag-and-drop, tables).

_Last updated: 2026-10-16._

## Analyses

| AccessMod 5 feature | Where in AM5 | accessmod-rs | Interface |
| --- | --- | --- | --- |
| **Accessibility**: travel time to/from facilities | `amAnalysisTravelTime.R` | ✅ `travel-time run`; matches AM5 to 1e-15 (anisotropic) and 2⁻²⁴ (isotropic) on the demo project | CLI |
| Isotropic / anisotropic | idem | ✅ | CLI |
| From / towards facilities | idem | ✅ | CLI |
| Knight's move (16 neighbours) | idem | ✅ | CLI |
| Walking (Tobler), bicycling (LUT), motorized | `mobility.h` | ✅ bicycle LUT bit-identical | CLI |
| Maximum travel time | idem | ✅ `limit_s` (seconds) | CLI |
| Nearest facility (cost allocation) | idem | ✅ `nearest_facility.tif` + ID table | CLI |
| Output in minutes, rounding (`ceil`/`round`/`floor`), min travel time, timeout value | `amCleanTravelTime` | 🗓 compatibility export; canonical output stays in seconds | CLI |
| Path distance, movement direction | `r.walk` `outdir` | ✅ `path_distance_m.tif`, `direction.tif` | CLI |
| Facility selection (subset of a layer) | `amFacilitiesSubset`, facility table | 🗓 attribute filter in the job; today: all features of the layer | CLI + UI (table selection) |
| Facilities on barriers: detected, user deselects | facility validation | ✅ listed as errors, or skipped with `facility_issues: skip` | CLI |
| Region cropping by max speed × time | `amSpeedBufferRegion.R` | ➖ not needed: the limit is applied exactly during the search | — |
| **Geographic coverage**: capacity-based population coverage, catchments, residual population | `amAnalysisCapacity.R`, `amAnalysisCatchment.R` | 🗓 plan: Later 1 | CLI + UI (results review) |
| **Zonal statistics**: population coverage by zone and time | `amAnalysisZonal.R` | 🗓 plan: Later 1 | CLI + UI (charts) |
| **Referral**: time and distance between facilities, nearest by time or distance, network | `amAnalysisReferralParallel.R`, `amAnalysisTimeDist.R` | ✅ `referral run`: many-to-many, nearest by time and distance; times equal AM5 (after its rounding) on the demo, distances within one cell ([`referral.md`](referral.md)); 🗓 network lines output | CLI |
| **Scaling up**: suitability, exclusion, new facility placement | `amAnalysisScalingUp.R` | 🗓 plan: Later 3 | CLI + UI (suitability tables) |
| **Optimization**: greedy best population coverage | `amAnalysisOptimization.R` | 🗓 plan: Later 3 | CLI |
| Replay of saved analyses | `amAnalysisReplay.R` | 🟡 `job.json` / `job.resolved.json` replay any run; 🗓 converter from AM5 analysis configs | CLI |

## Toolbox

| AccessMod 5 feature | Where in AM5 | accessmod-rs | Interface |
| --- | --- | --- | --- |
| **Merge land cover**: stack land cover, roads (classes from an attribute) and barriers (lines, areas) in priority order | `modules/amAnalysisMergeLandCover`, `amStackBarrier.R` | ✅ `merge-land-cover run`: stack as an ordered list; 2 cells of 44 492 differ from AM5 on the demo ([`merge-land-cover.md`](merge-land-cover.md)); 🗓 polygon skeleton option | CLI (stack as a list) + UI (ordering, conflicts) |
| Bridge and artefact cleaning when merging | `amBridgeTools.R` | ✅ `clean_bridges` (default on), `bridges.tif` identical to AM5 on the demo | CLI |
| Conflicting classes report | merge module | ✅ validation error naming the items; `reclass` on land-cover items (AM5 quick correction) | CLI + UI |
| **Adjust population distribution**: population on barriers, redistribution | `modules/amPopulationBarrierCorrection` | ✅ `adjust-population-distribution run`: zone or known population; equal to AM5 within 6e-15 on the demo ([`adjust-population.md`](adjust-population.md)) | CLI |
| **Interactive map**: preview layers, relocate facilities | `modules/amGisPreview`, toolbox | ➖ engine; 🗓 UI (plan: Later 5) | UI |

## Project and data

| AccessMod 5 feature | Where in AM5 | accessmod-rs | Interface |
| --- | --- | --- | --- |
| Project = GRASS location built from the DEM | `amProjectCreateFromDem` | 🟡 no project yet: the DEM of each job defines the grid, other rasters must lie on its lattice (stricter than AM5, which resamples on import) | CLI |
| Raster import with CRS / extent checks | `amUploadRaster.R` | ✅ validation at run time (CRS, resolution, origin, overlap); no implicit resampling or reprojection | CLI |
| Vector import (facilities) | `amUploadVector.R` | ✅ any GDAL point layer, explicit reprojection | CLI |
| Vector import (roads, barriers, zones) | idem | ✅ roads and barriers (any GDAL line or polygon layer, explicit reprojection); 🗓 zones with zonal statistics | CLI |
| Table import (scenarios, capacities) | `amUploadTable.R` | 🟡 scenario in the job JSON; 🗓 CSV scenarios and capacity tables | CLI + UI (table editing) |
| Data catalogue: tags, filter, rename, archive, delete | `modules/amManageData` | ➖ engine works on plain files; 🗓 UI | UI |
| Project export / import (`.am5p`) | `amProjectImportExport.R` | 🗓 separate converter (plan: Later 4) | CLI |
| Data export | `amExport.R` | ✅ outputs are GeoTIFF and JSON directly | CLI |
| Dataset inspection | data module | ✅ `dataset inspect` | CLI |
| Logs | `modules/amManageLogs` | ✅ `run.json` per run, stderr messages | CLI + UI (history) |
| Settings, languages (en, fr, es, pt) | `modules/amManageSettings`, `www/dictionary` | ➖ CLI flags; 🗓 UI (vocabulary follows the AM5 dictionary) | UI |
| Resource checks before running | `amGetRessourceEstimate` | ✅ pre-flight memory and disk estimate, `--memory` budget | CLI |

## Platform

| AccessMod 5 | accessmod-rs |
| --- | --- |
| R/Shiny web app + GRASS GIS + Docker, packaged as Electron app or VM | Single `accessmod` binary (CLI); 🗓 self-contained archives with GDAL/PROJ (milestone 1, distribution) |
| Interactive use only (plus replay) | Scriptable: job files, JSON on stdout, exit codes; 🗓 MCP server for agents (milestone 2) |
| Memory needs grow with the grid (GRASS segments) | Explicit memory budget, tiled disk storage, same results at any budget ([benchmarks](benchmarks.md)) |
| Results in minutes (rounded) | Results in seconds and metres; minutes as a compatibility export (🗓) |

## Where an interface is needed

The engine and CLI cover analyses end to end. An interface becomes
necessary for:

1. **Merge land cover**: ordering the stack, reviewing class conflicts and
   bridges.
2. **Scenario and capacity tables**: editing speeds and modes per class.
3. **Facility selection and relocation**: picking facilities from a table,
   moving misplaced points on a map.
4. **Result exploration**: maps, coverage charts, zonal tables.
5. **Data and project management**: catalogue, tags, history.

The plan keeps the UI last (Later 5), on top of the stabilised CLI and MCP
contracts, so every interface action maps to a reproducible job.

Feasibility, architecture and the screen-to-job mapping are in
[`ui.md`](ui.md).
