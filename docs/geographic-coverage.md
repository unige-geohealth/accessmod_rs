# Geographic coverage

Facilities take, one after the other, the population they can reach up to
their capacity: the AccessMod 5 geographic coverage (capacity) analysis
(`tools/R/amAnalysisCapacity.R`, `amAnalysisCatchment.R`). Status
(2026-10-16): implemented (`accessmod geographic-coverage`) and checked
against the AccessMod 5 demo replay `replay/examples/loop_capacity_stat`.

## AccessMod 5 today

1. Residual population: `pop > 0 &&& speed > 0 ? pop : 0` (no value without
   population or with people on a barrier, 0 for people <= 0).
2. Facilities ordered by a field (or by the population they reach, or a
   circular buffer).
3. For each facility: a travel time from it alone, rounded up to whole
   minutes; residual population summed per minute band; catchment by the
   cases A–E of `amCatchmentAnalyst` (inner ring of whole bands, part of
   the outer band); covered population removed; catchment polygon.
4. Table `tCapacityStat`, residual population raster, catchments.

## accessmod-rs approach

- The rule is `accessmod_core::capacity::catchment` (cases A–E, tested on
  the examples of the AccessMod 5 source).
- One search per facility on the shared solver; reached cells and their
  band (`ceil(t / band_s)`, 60 s by default as AccessMod 5) are spooled to
  a scratch file, so memory does not grow with catchments. Band sums read
  the residual population (a disk-backed grid); removal re-reads the spool.
- Catchments: the cells within the catchment band are burnt into a
  disk-backed mask over their bounding box, polygonised
  (`vector::polygonize`) and written as one MultiPolygon per facility
  (`vector::FeatureWriter`, FlatGeobuf).
- Outputs are byte-identical at any `--memory` (`tests/coverage.rs`).

## Contract

`accessmod geographic-coverage schema` prints the job. Main fields:
travel-time fields (`dem`, `land_cover`, `scenario`, `method`, `direction`,
`neighbourhood`, `limit_s`), `population`, `facilities` with
`capacity_field` and `name_field`, `order` (`{"field": {"field":
"capacity", "descending": true}}` or `{"population": {"limit_s": 7200,
"descending": true}}`), `band_s` (60), `ignore_capacity`,
`remove_covered` (true), `catchments` (true). Ties in the order follow the
facility identifiers.

Outputs: `capacity.csv` (one row per facility in processing order:
capacity, population within the limit, band correlation, catchment travel
time, population of the catchment, capacity realised and residual,
population not covered, cumulated covered percent, case),
`population_residual.tif`, `catchments.fgb`, `job.resolved.json`,
`run.json`.

## Comparison with AccessMod 5

`scripts/oracle/capacity-demo.R` replays `loop_capacity_stat` on the demo
(10 facilities, capacity descending, anisotropic towards facilities,
120 min); `tests/oracle_coverage.rs`:

| Output | Result |
| --- | --- |
| `capacity.csv` | every value equal to 1e-9 (catchment times × 60), same cases |
| residual population | same NoData mask, values equal to 5e-16 |
| catchments | same area for all 10 facilities |

## Deviations and gaps

- `dem_nodata_barrier`, as travel time.
- `coverage_order_ties`: equal order values follow facility identifiers.
- Not yet: circular-buffer order, population-on-barrier map, zonal
  coverage table (use `zonal-statistics` on the residual population).
