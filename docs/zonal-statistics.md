# Zonal statistics

Population covered within travel-time limits, per zone (for example per
district): the AccessMod 5 zonal statistics (`tools/R/amAnalysisZonal.R`).
Status (2026-10-16): implemented (`accessmod zonal-statistics`) and checked
against the AccessMod 5 demo project.

## AccessMod 5 today

1. Zones rasterised with `v.to.rast use=attr` (identifier and label
   fields).
2. For each limit (minutes, sorted, without duplicates): population where
   `0 <= travel time < limit` on the stored travel time (whole minutes,
   rounded up), summed per zone with `r.univar zones=`; also the population
   of each zone.
3. Table `zone, label, time_m, popTotal, popTravelTime, popCoveredPercent`,
   NA replaced by 0, ordered by coverage within each limit.

## accessmod-rs approach

Composed from the GIS toolkit: zones burnt by cell centre
(`vector::burn_zones`; features sharing an identifier form one zone, a cell
in several zones belongs to the first), then one pass over 256 × 256
blocks (`raster::for_each_block`) adding population and covered population
per zone and limit (`accessmod_core::zonal::ZoneCoverage`). Memory: 4 bytes
per cell (zone index, tiled under `--memory`), block buffers, per zone its
sums. The table is byte-identical at any budget (`tests/zonal.rs`).

## Contract

```json
{
  "schema_version": 1,
  "dem": "dem.tif",
  "travel_time": "run/travel_time_seconds.tif",
  "population": "population.tif",
  "zones": {"path": "districts.gpkg", "id_field": "code", "label_field": "name"},
  "time_limits_s": [1800, 3600, 7200],
  "output_dir": "zonal"
}
```

- `travel_time`: seconds, as written by `travel-time run`; NoData cells are
  not reached.
- A cell is covered within limit `L` when `0 <= t <= L` (inclusive, like the
  travel-time limit).

Output `zonal_statistics.csv`:
`zone_id,label,time_limit_s,population,population_covered,coverage_percent`,
limits ascending, zones by coverage ascending (ties by identifier, numbers numerically); 0 % for
a zone without population. `run.json` adds totals per limit and the
population outside the zones.

## Comparison with AccessMod 5

`scripts/oracle/zonal-statistics-demo.sh` exports the demo travel time
(minutes, limit 60), population and 4 zones, and the AccessMod 5 sums
within 30 and 60 minutes (`tests/oracle_zonal.rs`). With the same minutes
converted to seconds and limits `(L − 1) × 60 s` (equal to `t < L` on whole
minutes), population and covered population of every zone match to 1e-9.

## Deviations

- `zonal_limit_seconds`: limits are inclusive, in seconds, on unrounded
  times; AccessMod 5 compares minutes rounded up with `t < limit`.
- `overlapping_zones`: a cell in several zones belongs to the first zone in
  layer order; `v.to.rast` keeps the last one.
