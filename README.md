# accessmod-rs

Portable rewrite of [AccessMod](https://github.com/unige-geohealth/accessmod):
physical accessibility to health services (travel time, nearest facility,
coverage, catchments) as a standalone engine, without GRASS GIS, R or Docker.

This engine is the future reference implementation of AccessMod and is meant to
replace the AccessMod 5 stack (R/Shiny + GRASS + Docker). Until it reaches
feature parity, AccessMod 5 is used **only as a numerical oracle**: its fixtures
and outputs validate this engine; its code is not copied.

> Status: pre-alpha. The travel-time analysis works end to end from the
> command line and matches AccessMod 5; other analyses are not ported yet
> ([coverage](docs/coverage.md)). Contracts may still change.

## Quick start

```sh
cargo build --release -p accessmod-cli     # needs GDAL ≥ 3.5 and PROJ
./target/release/accessmod doctor
./target/release/accessmod travel-time run --job docs/tutorial/data/job.json
```

The [tutorial](docs/tutorial/README.md) walks through inputs, job, outputs
and figures on a small synthetic landscape.

## AccessMod 5 vs accessmod-rs

Same travel-time analysis (anisotropic, 16 neighbours, three transport
modes) on synthetic grids, single runs on an Apple M1 Pro
([details and caveats](docs/benchmarks.md)):

| Cells | AccessMod 5 (`r.walk.accessmod`) | accessmod-rs (solve / total) | Memory AM5 → rs |
| ---: | ---: | ---: | ---: |
| 1 M | 0.96 s | 0.78 s / 1.5 s | 216 → 136 MB |
| 16 M | 21.8 s | 16.7 s / 18.6 s | 2.1 → 0.7 GB |
| 64 M | 121.7 s | 92.1 s / 100.2 s | 7.9 → 1.9 GB |
| 64 M, `--memory 1G` | — | 101.3 s / 117.5 s | 0.85 GB |

Referral (all facilities to all facilities, [details](docs/benchmarks.md)):

| Cells, facilities | AccessMod 5 | accessmod-rs |
| ---: | ---: | ---: |
| demo 44 k, 10 | 13.1 s (replay analysis) | 0.3 s |
| 16 M, 100 (9 900 pairs) | ≥ 72.6 s (engine only, lower bound) | 17.7 s |

- **Same numbers**: travel times agree with AccessMod 5 to 1e-15 (relative),
  with identical reachable areas and nearest facilities; on the AccessMod 5
  demo project as well ([oracle tests](docs/plan.md)).
- **Less memory, bounded memory**: 1.6 × (1 M cells) to 4 × (64 M) less than
  AccessMod 5, and a hard
  budget (`--memory`) with disk-backed tiles: results are byte-identical
  whatever the budget, so national grids run on small machines.
- **Faster engine**: the solver is 16–24 % faster than `r.walk.accessmod`;
  the total includes reading inputs and writing compressed outputs, which
  AccessMod 5 does in separate import and export steps.
- **One binary**: no R, Shiny, GRASS or Docker; scriptable with job files,
  JSON output and exit codes.
- **Referral without the detours**: distances come from the search itself (no
  `r.path` / `v.net`), searches stop at the last destination and run in
  parallel; many-to-one runs from the few destinations.
- **No start-up cost**: an AccessMod 5 replay spends 6 s loading its R code
  before computing; the demo travel time takes 9 s end to end in AccessMod 5
  and 0.5 s with accessmod-rs.

## Deliverables (planned)

| Milestone | Deliverable |
| --- | --- |
| 1 | `accessmod` CLI: complete travel-time engine on disk-backed tiled rasters (isotropic/anisotropic, nearest, path distance) |
| 2 | MCP stdio server + agent skill over the same public API |
| 3+ | Zonal stats, coverage, referral, optimization, project import |
| last | Graphical UI on top of the stabilised CLI/MCP contracts |

## Documentation

- [`docs/tutorial/`](docs/tutorial/README.md) — step-by-step travel-time analysis
- [`docs/coverage.md`](docs/coverage.md) — AccessMod 5 features: covered, planned, needing a UI
- [`docs/benchmarks.md`](docs/benchmarks.md) — performance against AccessMod 5
- [`docs/cost-model.md`](docs/cost-model.md) — cost model, neighbours and direction codes
- [`docs/plan.md`](docs/plan.md) — architecture, contracts and milestones
- [`docs/reference-accessmod5.md`](docs/reference-accessmod5.md) — where the
  oracle behaviour lives in the AccessMod 5 repository
- [`AGENTS.md`](AGENTS.md) — workflow, commands, conventions (humans and agents)
- [`STYLE.md`](STYLE.md) — Rust style guide

## License

LGPL-3.0-or-later. See [`LICENSE`](LICENSE) (LGPL-3.0) and [`COPYING`](COPYING) (GPL-3.0, which the LGPL supplements).
