# Benchmarks

Indicative timings; no performance promise before a baseline on both target
profiles (`plan.md`). Reproduce with:

```sh
cargo run --release -p accessmod-core --example synthetic -- <side> <tile> <capacity> [16]
```

Synthetic landscape: anisotropic, walking / bicycling / motorized classes on a
rolling DEM, 2 % barriers, 4 facilities spread over the grid, 8 neighbours.
`capacity` is the number of resident tiles per grid (inputs and state).

## 2026-10-09 — Apple M1 Pro, 32 GB, single-threaded solver

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
