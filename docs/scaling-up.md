# Scaling up

Add facilities one at a time where they are most suitable and cover the
most people: the AccessMod 5 scaling-up analysis
(`tools/R/amAnalysisScalingUp.R`). Status (2026-10-16): implemented
(`accessmod scaling-up`) and equal to the AccessMod 5 demo replay
`replay/examples/demo_scaling_up`.

## AccessMod 5 today

1. Residual population as in the geographic coverage; candidates are the
   cells with a speed (or friction) value.
2. At each iteration, up to the facility, coverage or time limit:
   - exclusion rules remove candidates: within or beyond a buffer
     (`r.buffer`) around a layer, the facilities added so far included;
   - suitability factors are computed and rescaled over the candidates to
     0–10 000 (`amRasterRescale`): population summed over a moving window
     (`r.neighbors -c`), Euclidean distance (`r.grow.distance`), travel time
     from a layer (whole minutes), a priority raster; high or low values
     favoured; the suitability is `int(Σ weighted / Σ weights)`;
   - every cell of maximum suitability is evaluated: one travel time from
     it (120 min in the demo), the residual population reached;
   - the cell reaching the most people becomes a facility; the capacity
     table gives its capacity from that population; its catchment
     (`amCatchmentAnalyst`) is removed from the residual population. A cell
     reaching no one is removed from the candidates instead.
3. Outputs: facilities (existing and new), capacity table, residual
   population, catchments.

## accessmod-rs approach

- The rules (rescale, capacity class, suitability) are in
  `accessmod_core::scaling_up`; the catchment steps are shared with the
  geographic coverage (`accessmod_io::catchment`).
- New toolkit blocks, written for this module and reusable:
  `accessmod_core::focal::distance_to_sources` (exact Euclidean distance
  transform, two separable passes, row by row) and `circular_sum` (disc of
  `dr² + dc² <= r²`, NaN skipped, as `r.neighbors -c`), and
  `vector::Burn::Thin`: points and thin 8-connected lines as `v.to.rast`
  without `-d`, equal cell for cell to GRASS on the demo roads
  (`tests/oracle_grass_toolkit.rs`, with the distance transform against
  `r.grow.distance`).
- Ties fall as in AccessMod 5 because its arithmetic is reproduced: R
  prints minimum, maximum and weights with 15 significant digits into
  `r.mapcalc`; travel times are integer minutes, so their rescale uses
  integer division; distances equal `r.grow.distance` to the bit
  (`tests/oracle_grass_toolkit.rs`).
  Cells of maximum suitability are evaluated in row-major order and the
  first reaching the most people wins (`which.max`).
- Facility distances are computed once with the distance transform, then
  lowered for each new facility; static layers (exclusions, factors from
  rasters or vectors) are prepared once; the population window and
  facility-based travel times are recomputed at each iteration.
- Every grid is a disk-backed tiled grid counted in the memory plan;
  outputs are byte-identical at any `--memory` (`tests/scaling_up.rs`).

## Contract

`accessmod scaling-up schema` prints the job. Main fields:

- travel-time fields of the candidate evaluations and catchments (`dem`,
  `land_cover`, `scenario`, `method`, `direction` (towards facilities by
  default, as AccessMod 5), `neighbourhood`, `limit_s`, `band_s` (60));
- `population` (the covered percent refers to it), `population_residual`
  (the population to cover at start; `population` when omitted),
  `facilities` (existing facilities kept);
- `capacities`: `[{"min", "max", "capacity", "label"}]`; a population in
  no range, or in several, takes the smallest or largest capacity,
  whichever is nearer;
- `exclusions`: `[{"layer", "buffer_m", "keep": "inside" | "outside"}]`;
- `suitability`: factors with `weight` and `prefer` (`high` | `low`):
  `{"factor": "population", "layer": "population" | {"raster": …},
  "window_m"}`, `{"factor": "distance", "layer": …}`,
  `{"factor": "travel_time", "layer": …, "method", "direction",
  "neighbourhood"}`, `{"factor": "priority", "layer": {"raster": …}}`;
- layers: `"facilities"` (existing and added), `"population"` (residual),
  `{"raster": path}` (cells with a value), `{"vector": {"path", "layer",
  "where"}}` (points, thin lines, polygons);
- limits: `max_facilities`, `max_coverage_percent` (100),
  `max_duration_s` (none; set, it makes results depend on the machine);
  `catchments` (true).

`window_m` keeps AccessMod 5's meaning of its option `r`: the window is
`round(window_m / cell)` cells wide, made odd, so its radius is half of it
(the demo's `r = 1 km` on 1 km cells is the cell alone).

Outputs: `capacity.csv` (one row per facility added: order, iteration id,
`facility_<id>`, class, capacity, suitability, then the catchment columns
of the geographic coverage), `facilities.fgb` (existing facilities at their points, `added` 0, then
the new ones at cell centres, `added` 1, as the AccessMod 5 output layer), `population_residual.tif`, `catchments.fgb`,
`job.resolved.json`, `run.json` (iterations, facilities added, candidates
evaluated, stop reason, totals).

## Comparison with AccessMod 5

`scripts/oracle/scaling-up-demo.R` replays `demo_scaling_up` on the demo
(10 existing facilities, exclusion 5 km around facilities, population,
distance to facilities and isotropic travel time from roads, 120 min,
10 new facilities). The example's population `rPopulation__cor_pop` is not
in the image; `rPopulation__demo_patients` is used.
`tests/oracle_scaling_up.rs` runs `benchmark/scaling-up/demo-job.json`:

| Output | Result |
| --- | --- |
| `capacity.csv` | same 10 facilities, classes and capacities; every value equal to 1e-9 |
| facilities | same 20 (10 existing, 10 new); new ones on the same cells |
| residual population | same NoData mask, values equal to 1e-9 |
| catchments | same polygons for all 10 (equal areas, intersection covering both) |

AccessMod 5 takes 39.4 s for this replay, accessmod-rs 0.50 s
([benchmark](../benchmark/scaling-up/README.md)).

## Deviations and gaps

- `dem_nodata_barrier`, as travel time.
- `scaling_up_limits`: limits are used as given (AccessMod 5 treats a
  coverage limit above 99 % as 100 %).
- Sources of a travel-time factor on barriers start nothing (`r.cost`
  skips them).
- The population window sums in double precision; `r.neighbors` writes
  single precision for a single-precision population.
