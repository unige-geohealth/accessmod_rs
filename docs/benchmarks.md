# Benchmarks

## AccessMod 5 vs accessmod-rs — travel time

Reproduce with `sh scripts/bench/am5-vs-rs.sh [side ...]` (needs Docker and the
pinned AccessMod 5 image). Each grid is `side × side` cells of 100 m generated
by `synthetic_inputs` (the tutorial landscape scaled up), 10 facilities, all
three transport modes; anisotropic, towards facilities, 16 neighbours, no
limit.

2026-10-09, Apple M1 Pro (10 CPUs, 32 GB):

| Cells | AM5 `r.walk.accessmod` | AM5 peak RSS | accessmod-rs total | accessmod-rs solve | accessmod-rs peak RSS | Max rel. diff |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 M | 0.96 s | 216 MB | 1.52 s | 0.78 s | 136 MB | 4.3e-16 |
| 4 M | 4.09 s | 583 MB | 4.14 s | 3.43 s | 262 MB | 4.3e-16 |
| 16 M | 21.8 s | 2 058 MB | 18.6 s | 16.7 s | 730 MB | 6.9e-16 |
| 64 M | 121.7 s | 7 947 MB | 100.2 s | 92.1 s | 1 920 MB | 1.3e-15 |
| 64 M, `--memory 1G` | — | — | 117.5 s | 101.3 s | 854 MB | identical bytes |

Reachable masks are identical in every case.

How to read it:

- **Same results**: travel times agree to floating-point rounding.
- **Memory**: accessmod-rs needs 1.6 × (1 M cells) to 4 × (64 M) less memory,
  the gap growing with the grid, and honours a budget:
  with `--memory 1G` the 64 M-cell grid runs in 854 MB (tiles spilled to
  disk), 17 % slower, with byte-identical outputs. `r.walk.accessmod` was
  given 8 000 MB (`memory=`) so that it never spills.
- **Speed**: the solver is 16–24 % faster than `r.walk.accessmod` from 1 M
  cells. The accessmod-rs *total* includes everything a user waits for:
  reading the GeoTIFFs, input checksums, the solve, writing four compressed
  GeoTIFFs and the reports. The AccessMod 5 figure is the GRASS module alone;
  a real AccessMod 5 analysis adds the R/Shiny orchestration, speed-map
  creation, the minute conversion (`amCleanTravelTime`) and requires the
  data to be imported in a GRASS project first.
- **Caveats**: AccessMod 5 runs in its Docker image (native arm64 Linux in
  the Docker VM, 20.9 GB, 10 CPUs); accessmod-rs runs natively on macOS.
  Single runs, warm caches; not a substitute for the baselines on both
  target profiles required by the plan.

## End to end: AccessMod 5 replay vs accessmod-rs

What a user waits for, from command to results, on the AccessMod 5 demo
project (227 × 196 cells, 10 facilities). AccessMod 5 runs a replay
configuration in its Docker image (`Rscript` sourcing `global.R`, then
`amAnalysisReplayExec`); accessmod-rs runs the CLI on the same data exported
as GeoTIFF / FlatGeobuf.

| Analysis | AccessMod 5 container total | of which loading `global.R` | analysis + export | accessmod-rs |
| --- | ---: | ---: | ---: | ---: |
| Travel time (`config_demo_motorized.json`) | 9 s | 6.1 s | 2.5 s | 0.5 s |
| Referral 10 × 10 (`replay/examples/demo_referral`) | 24 s | 6.0 s | 17.2 s | 0.3 s (solve 0.02 s) |

On small and medium grids the fixed cost of the AccessMod 5 stack (container
start, R packages, `global.R`, GRASS sessions) dominates; accessmod-rs has no
such cost. Replays need `TZ` set in the container (`/etc/timezone` is
missing from the image).

## Referral: AccessMod 5 vs accessmod-rs

All facilities to all facilities (pairs = N × (N − 1)) on the synthetic
landscape, anisotropic, 16 neighbours. Reproduce with
`sh scripts/bench/referral-vs-am5.sh side facilities [...]`.

The AccessMod 5 column is a **lower bound** of its referral: only the
engine part, one `r.walk.accessmod` per origin stopped at the destinations
(`stop_points`), 9 at a time in its Docker image. A real AccessMod 5
referral also traces every path with `r.path`, builds a vector network
(`v.net`) and measures it (`v.net.distance`), and runs under R: on the
demo project the full replay takes 13–15 s of analysis for 90 pairs.

2026-10-09, Apple M1 Pro (10 CPUs):

| Cells | Facilities | Pairs | AM5 engine lower bound | accessmod-rs total | accessmod-rs solve |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 M | 20 | 380 | 1.3 s | 0.8 s | 0.16 s |
| 4 M | 50 | 2 450 | 7.6 s | 2.1 s | 1.7 s |
| 16 M | 100 | 9 900 | 72.6 s | 17.7 s | 16.6 s |
| demo, 44 k | 10 | 90 | 13.1 s (full replay analysis) | 0.3 s | 0.02 s |

Notes:

- A first 16 M run gave 135.8 s for AccessMod 5 (9 parallel
  `r.walk.accessmod` with 2 GB each in a 20.9 GB VM, likely memory
  pressure); the table keeps the faster run.
- Times agree with AccessMod 5 on the demo referral (all 180 pairs equal
  after AM5's rounding up to whole seconds); distances differ by less than
  one cell diagonal (cell centres vs `r.path` lines from the points).
- Fewer destinations than origins (e.g. many health centres to a few
  hospitals) costs only as many searches as destinations
  (`search_from: auto`); `nearest_only` stops each search early.

## Solver scaling (synthetic example)

Indicative timings; no performance promise before a baseline on both target
profiles (`plan.md`). Reproduce with:

```sh
cargo run --release -p accessmod-core --example synthetic -- <side> <tile> <capacity> [16]
```

Synthetic landscape: anisotropic, walking / bicycling / motorized classes on a
rolling DEM, 2 % barriers, 4 facilities spread over the grid, 8 neighbours.
`capacity` is the number of resident tiles per grid (inputs and state).

## 2026-10-09 — Apple M1 Pro, 32 GB, single-threaded solver

Measured before the solver optimisations of the same day (input window per
settled cell, store fast path), which cut solve time by about 20 %.

Grid 2000 × 2000 (4 M cells), solve time (excluding input build):

| Tile | Resident tiles | Resident memory | Solve | State tiles read back |
| ---: | ---: | ---: | ---: | ---: |
| 256 | 64 / 64 | 168 MB | 2.3 s | 0 |
| 256 | 8 / 64 | 21 MB | > 10 min (thrashing, aborted) | — |
| 64 | 1000 / 1024 | 164 MB | 2.3 s | 0 |
| 64 | 400 / 1024 | 65 MB | 2.4 s | 0 |
| 64 | 200 / 1024 | 33 MB | 2.4 s | 0 |
| 64 | 100 / 1024 | 17 MB | 18.5 s | 413 k |
| 64 | 50 / 1024 | 8 MB | 88 s | 2.3 M |
| 32 | 100 / 3969 | 4 MB | 29.6 s | 2.4 M |

Findings:

- About 580 ns per cell, CPU-bound, independent of the regime as long as the
  frontier band fits: each state tile is then written once and never read
  back.
- Below the frontier working set, the LRU cache degrades progressively
  (not a hang), with identical results.
- Small tiles (64²) fit the frontier band far better than 256² tiles; 64 is
  the default (`store::DEFAULT_TILE`).
- Open: the working set grows with the frontier length, i.e. with the number
  and spread of facilities; to measure on realistic national grids. Solver
  CPU cost has not been profiled yet.
