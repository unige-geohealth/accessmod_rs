# Referral analysis — design (draft, 2026-10-09)

Travel time and path distance between facilities: every origin facility to
every destination facility (many-to-many), the nearest destination by time
and by distance, and optionally the paths as a network. Priority: next
analysis after the travel-time milestone.

## AccessMod 5 today

`tools/R/amAnalysisReferralParallel.R`, `amAnalysisTimeDist.R`:

1. One job per origin facility, run in R parallel workers.
2. Each job runs a **full travel-time raster** (`r.walk.accessmod` or
   `r.cost`) from that single origin over the whole region, then samples it
   at the destinations.
3. Distances: `r.path` traces the path from each destination back to the
   origin on the direction raster, `v.net` builds a vector network and
   `v.net.distance` measures it.
4. Outputs: `tReferral` (origin, destination, time, distance),
   `tReferralTime` and `tReferralDist` (nearest destination by time and by
   distance), `vReferralNetwork` (paths), in minutes and km.

Cost: N full-grid searches plus, per pair, path tracing and network
building, plus GRASS and R overhead per job. The demo replay (10 × 10
facilities, 44 k cells) takes 17 s of analysis; the cost grows as
N × grid size.

## accessmod-rs approach

**One exact search per source, stopped early, sources in parallel.**

- **Distances come free.** The solver already accumulates the planar length
  of the least-time path (`path_distance_m`). No path tracing, no vector
  network: time and distance at a destination are read when it is settled.
- **Early stop.** A search ends as soon as every destination is settled, or
  the k nearest (`nearest_only` = AM5 `limitClosest`), or the limit is
  reached. Searches never cover more than they need.
- **Fewer searches.** The search runs from whichever side is smaller:
  `time(o → d)` with `from` semantics equals the search from `d` with
  `towards` semantics (slopes inverted, same cells, same knight cells), so
  N × M pairs need `min(N, M)` searches. The swap is automatic and recorded
  in `run.json` (it can change the last bits of floating sums: knight-cell
  summation order differs).
- **Parallel and deterministic.** Searches are independent: run on
  `--threads` workers sharing the inputs read-only; each worker has its own
  sparse state (tiles are allocated only where the search goes). Results are
  collected and sorted by (origin order, destination order), so output never
  depends on scheduling.
- **Paths on demand.** With `outputs.network`, paths are rebuilt from the
  predecessor steps (`direction` codes) as lines, written to a GeoPackage.

No single-pass exact many-to-many exists on this graph (costs are
asymmetric and per source); the gain comes from early stopping, the smaller
side, parallelism and dropping the raster / vector round trips.

## Contract sketch

Job (`referral`, schema version 1): DEM, land cover, scenario, method,
direction, neighbourhood, limit as in the travel-time job, plus

```json
"origins": { "path": "hf.gpkg", "id_field": "code" },
"destinations": { "path": "hospitals.gpkg", "id_field": "code" },
"nearest_only": false,
"outputs": { "network": false }
```

Outputs:

- `referral.csv` / `.json`: origin id, destination id, `time_s`,
  `distance_m`, reached flag; one row per pair, sorted;
- `nearest_by_time.csv`, `nearest_by_distance.csv`;
- `network.gpkg` (optional): one line per pair along the least-time path;
- `run.json`, `job.resolved.json` as for travel time.

## Validation

- Oracle: the AM5 demo referral replay (`replay/examples/demo_referral`),
  times and distances per pair; tolerance to characterise (AM5 distances
  come from `r.path` lines through cell centres).
- Property: referral times equal travel-time rasters sampled at the
  destinations; swapping sides gives the same pairs to 1e-12.
- Benchmarks: N × M on synthetic grids against the AM5 replay.

## Open questions

- Origins and destinations from one layer with a type filter (AM5 uses two
  selections of the same layer)?
- Pairs with the same facility (distance 0): kept or dropped?
- Output units: seconds and metres (canonical) plus the AM5 minutes / km
  compatibility export?
