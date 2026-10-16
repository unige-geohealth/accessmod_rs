# Optimization

Rank existing facilities by the population their catchments add to the
coverage: the AccessMod 5 optimization analysis
(`tools/R/amAnalysisOptimization.R`). Status (2026-10-16): implemented
(`accessmod optimization`) and equal to AccessMod 5 on its demo tests
(`tests/optimization`) and on overlapping discs.

## AccessMod 5 today

1. Inputs: catchment polygons (a geographic coverage output) with the
   identifier of their facility, a population raster (it sets the region),
   the number of facilities to select; optionally a minimum number of
   facilities per administrative zone, with the zones and the facility
   points.
2. Catchments with identical geometries are kept once and reported
   together (`A // B`). The population of each catchment is extracted with
   `exactextractr` (`sum`: each cell weighted by the fraction of it the
   polygon covers).
3. Greedy loop, up to the number requested: the catchment covering the
   most remaining population wins (ties: the largest whole catchment, then
   the first); its geometry is subtracted (`st_difference`) from every
   catchment it intersects, whose population is extracted again. A
   catchment left empty is set aside; candidates covering no one are
   dropped after a step that reduced some catchment. With the zone minimum,
   candidates of the zones still lacking facilities compete first.
4. When fewer catchments than requested were selected, those set aside
   (entirely inside selected ones) are appended, covering nothing.
5. Output: one table (rank, facility, catchment population, population
   covered, zone, cumulative population covered).

## accessmod-rs approach

- The selection rules (tie-breaks, zone minimum) are in
  `accessmod_core::optimization`.
- New toolkit blocks, reusable by other analyses:
  `accessmod_core::raster::polygon_coverage`, the exact fraction of each
  cell covered by a polygon (Green's theorem, integrated in closed form on
  each edge, row by row), and `accessmod_io::vector::covered_sum`, the sum
  of a raster weighted by those fractions (`exactextractr` `sum`);
  `vector::GeometrySpool`, geometries spooled to disk as WKB and compacted;
  `GridSpec::open_raster`, the grid of an analysis without a DEM.
- Catchments are read once, reprojected to the grid CRS and spooled; memory
  holds one small record per catchment and the few geometries of one step
  (selected, candidate, difference). Differences are computed by GEOS
  through GDAL, as `sf` does; only catchments whose envelopes meet the
  selected one are tested.
- The run is sequential and does not depend on `--memory` or `--threads`
  (`tests/optimization.rs`).

## Contract

`accessmod optimization schema` prints the job. Fields:

- `population`: population raster; it defines the grid;
- `catchments`: `{"path", "layer", "id_field", "where"}`, polygons whose
  `id_field` holds the identifier of their facility (`catchments.fgb` of a
  geographic coverage, with `facility_id`);
- `max_facilities`: facilities to select;
- `zone_quota` (optional): `{"zones": {"path", "layer", "id_field",
  "label_field", "where"}, "facilities": {"path", "layer", "id_field",
  "where"}, "min_per_zone"}`. A catchment belongs to the zone of its
  facility point (the last zone in layer order holding it, as AccessMod
  5); every catchment needs a facility with the same identifier (`4`,
  `4.0` and `"4"` match). Only zones holding catchments get the minimum;
  zones times `min_per_zone` must not exceed `max_facilities`.

Outputs: `optimization.csv` (`rank`, `facility_id`, `facility_ids` (the
identifiers of all catchments sharing its geometry, ` // ` separated),
`population_catchment`, `population_covered`, `zone_id` and `zone_label`
with a zone minimum, `population_covered_cumulative`), `job.resolved.json`,
`run.json` (catchments, duplicates, facilities selected, population
covered, differences computed, stop reason).

## Comparison with AccessMod 5

`scripts/oracle/optimization-demo.R` runs `tests/optimization/config.json`
of AccessMod 5 on the demo (10 catchments of `vCatchment__demo` on
`rPopulation__demo_patients`, 10 facilities, one per zone of
`vZone__demo`), the same without the zone minimum, and 15 km discs around
the 10 demo facilities, which do not follow the cells and overlap.
`tests/oracle_optimization.rs` runs `benchmark/optimization/demo-job.json`
and its variants:

| Case | Result |
| --- | --- |
| zone minimum | same facilities, ranks and zones; values equal in single precision |
| no zone | same facilities and ranks; values equal in single precision |
| discs | same facilities and ranks; values equal in single precision |

AccessMod 5 takes 4.24 s for the demo test, accessmod-rs 0.29 s; 58.11 s and
0.42 s on 672 overlapping discs selecting 100 facilities.

AccessMod 5 returns each extracted population in single precision
(`exactextractr` on a Float32 raster): its values are ours rounded to
float32 (within 2⁻²³ relative); accessmod-rs keeps double precision. On the
benchmark's 672 overlapping discs the ranking is the same
([benchmark](../benchmark/optimization/README.md)).

## Deviations and gaps

- `optimization_duplicates`: catchments sharing a geometry are those with
  the same coordinates; AccessMod 5 names its groups with `st_equals`
  (topological equality) but keeps one catchment per identical coordinates.
- `optimization_zone_quota`: while a zone lacks facilities only candidates
  of such zones compete (AccessMod 5 takes their best value, then any
  candidate with that value); when they have none left, every candidate
  competes (AccessMod 5 fails); zones are told apart by identifier
  (AccessMod 5: by label); a facility in no zone counts for no zone and
  keeps its row (AccessMod 5 drops the row from the table).
- Populations are not rounded to single precision: ties that only exist
  after that rounding may break differently.
