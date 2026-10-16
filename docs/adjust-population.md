# Adjust population distribution

Population lying on barriers (cells where the merged land cover is NoData:
rivers, lakes, …) can never be reached. The AccessMod 5 toolbox "Adjust
population distribution" moves it to the rest of its zone, or scales each
zone to a known population. Status (2026-10-16): implemented
(`accessmod adjust-population-distribution`) and checked against the
AccessMod 5 demo project.

## AccessMod 5 today

`modules/amPopulationBarrierCorrection/helper.R`:

1. Partial population: the population where the land cover is not null.
2. Per zone (`v.rast.stats`, sum): full and partial population.
3. `ratio = full / partial`, or `known / partial` when a zone field holds
   the known population.
4. Zones rasterised with their ratio (`v.to.rast use=attr`, cell centre);
   output = partial population × ratio.
5. Summary: population before and after, outside zones, on barriers.

Cells outside every zone, on barriers, or in a zone without population off
barriers get no population (null).

## accessmod-rs approach

Composed from the GIS toolkit (`AGENTS.md`):

- zones burnt by cell centre into a disk-backed grid of zone indices
  (`vector::burn_layer`, `Burn::Polygons`); a cell in several zones belongs
  to the first zone in layer order;
- two passes over 256 × 256 blocks (`raster::for_each_window`): totals per
  zone, then the adjusted population, written block by block;
- the rule itself, `ratio = target / population off barriers`, is
  `accessmod_core::population::ZoneTotals`.

Memory: 4 bytes per cell (zone index, tiled under `--memory`), block
buffers, and a few hundred bytes per zone. Outputs are byte-identical at any
budget (`tests/population.rs`).

## Contract

Job (`accessmod adjust-population-distribution schema` prints it):

```json
{
  "schema_version": 1,
  "dem": "dem.tif",
  "population": "population.tif",
  "land_cover": "land_cover_merged.tif",
  "zones": {"path": "districts.gpkg", "id_field": "name",
            "population_field": "census_2020"},
  "output_dir": "population"
}
```

- `population`: any numeric raster on the DEM lattice (people per cell).
- `land_cover`: NoData cells are barriers (use the merged land cover).
- `zones.id_field`: unique identifier; `population_field` (optional): known
  population of each zone, a number ≥ 0.

Outputs:

| File | Content |
| --- | --- |
| `population_adjusted.tif` | Float64 people per cell; NoData −9999 on barriers, outside zones and in zones that cannot be placed |
| `zones.csv` | `zone_id,cells,population,population_on_barrier,population_known,ratio,population_adjusted` |
| `job.resolved.json`, `run.json` | Totals: population before and after, on barriers, outside zones, unplaced; overlapping cells; warnings |

## Comparison with AccessMod 5

`scripts/oracle/population-correction-demo.sh` runs the GRASS commands of
the toolbox on the demo project (population of patients, merged land cover,
4 zones), with the zone population and with a known population per zone
(`tests/oracle_population.rs`):

| Case | Cells | Largest relative difference | Zone ratios |
| --- | --- | --- | --- |
| zone population | same NoData mask | 5.8e-15 | equal to 1e-12 |
| known population | same NoData mask | 4.6e-15 | equal to 1e-12 |

Differences come from the summation order only.

## Deviations

Listed in `run.json` (`deviations`):

- `population_on_lattice`: the population raster must lie on the DEM
  lattice; AccessMod 5 resamples rasters when importing them.
- `overlapping_zones`: a cell in several zones belongs to the first zone in
  layer order; AccessMod 5 builds a GRASS topology, which splits overlaps.
