# accessmod-rs

Portable rewrite of [AccessMod](https://github.com/unige-geohealth/accessmod):
physical accessibility to health services (travel time, nearest facility,
coverage, catchments) as a standalone engine, without GRASS GIS, R or Docker.

This engine is the future reference implementation of AccessMod and is meant to
replace the AccessMod 5 stack (R/Shiny + GRASS + Docker). Until it reaches
feature parity, AccessMod 5 is used **only as a numerical oracle**: its fixtures
and outputs validate this engine; its code is not copied.

> Status: pre-alpha. Travel time and referral work end to end from the
> command line and match AccessMod 5 on its demo project; other analyses are
> not ported yet ([coverage](docs/coverage.md)). Contracts may still change.

## Quick start

```sh
cargo build --release -p accessmod-cli     # needs GDAL ≥ 3.5 and PROJ
./target/release/accessmod doctor
./target/release/accessmod travel-time run --job docs/tutorial/data/job.json
./target/release/accessmod referral run --job docs/tutorial/data/referral.json
```

The [tutorial](docs/tutorial/README.md) walks through inputs, jobs, outputs
and figures on a small synthetic landscape (travel time, then referral).

### Web interface (in progress)

The API runs with `accessmod serve` (http://127.0.0.1:5099; projects in
`~/AccessMod`, `--data` or `ACCESSMOD_DATA` to change). The client is being
rebuilt on the design system in [`design_system/`](design_system/README.md):
AccessMod 5 look and layout, light and dark, en/fr/es/pt. Its gallery:

```sh
npm --prefix design_system/design-system ci   # needs Node 24
npm --prefix design_system/design-system run dev   # http://localhost:5180
```

Design: [`docs/ui.md`](docs/ui.md).

## AccessMod 5 vs accessmod-rs

Median of 3 runs on an Apple M1 Pro, AccessMod 5 in its Docker image
([all modules, scripts and reports](benchmark/README.md)). On the AccessMod 5
demo project, full analysis (AccessMod 5 replay, without its 5.4 s of R
start-up, against the accessmod-rs command):

| Module | AccessMod 5 | accessmod-rs |
| --- | ---: | ---: |
| Travel time | 1.42 s | 0.24 s |
| Referral, 90 pairs | 11.67 s | 0.23 s |
| Geographic coverage | 14.55 s | 0.40 s |
| Scaling up, 10 facilities added | 39.39 s | 0.50 s |
| Optimization, 10 facilities, 1 per zone | 4.24 s | 0.29 s |

On 16 M-cell synthetic grids (AccessMod 5: its engine alone, a lower bound):

| Module | AccessMod 5 | accessmod-rs | Memory AM5 → rs |
| --- | ---: | ---: | ---: |
| Travel time, 10 facilities | 15.8 s | 16.1 s (solve 14.7 s) | 2.1 → 0.7 GB |
| Referral, 9 900 pairs | ≥ 35.7 s | 13.5 s | — → 0.7 GB |
| Geographic coverage, 100 facilities | ≥ 133.9 s | 19.7 s | — → 0.5 GB |

- **Same numbers**: travel times agree with AccessMod 5 to 1e-15 (relative),
  with identical reachable areas and nearest facilities; on the AccessMod 5
  demo project as well ([oracle tests](docs/plan.md)). Referral times match
  AccessMod 5 on its demo after its rounding up to whole seconds; distances
  differ by less than one cell ([why](docs/referral.md)).
- **Less memory, bounded memory**: about 3 × less than AccessMod 5 at 16 M
  cells, and a hard budget (`--memory`) with disk-backed tiles: results are
  byte-identical whatever the budget, so national grids run on small
  machines.
- **Engine on par, analyses faster**: the travel-time solver matches
  `r.walk.accessmod` (its total includes reading inputs and writing
  compressed outputs); analyses built on many searches gain most, as
  accessmod-rs works on the reached cells only.
- **One binary**: no R, Shiny, GRASS or Docker; scriptable with job files,
  JSON output and exit codes.
- **Referral without the detours**: distances come from the search itself (no
  `r.path` / `v.net`), searches stop at the last destination and run in
  parallel; many-to-one runs from the few destinations.
- **No start-up cost**: an AccessMod 5 replay spends about 5 s loading its R
  code before computing.

## Deliverables

| Milestone | Deliverable | Status |
| --- | --- | --- |
| 1 | `accessmod` CLI: travel-time engine on disk-backed tiled rasters (isotropic/anisotropic, nearest, path distance) | done |
| 2 | Every analysis and tool as a CLI job: referral, zonal statistics, geographic coverage, scaling up, optimization, land-cover merge, population adjustment | done (referral network output to do) |
| 3 | Project layer and web interface (`accessmod serve`), desktop or country server, in testable slices | next |
| 4 | Distribution: self-contained archives with GDAL/PROJ, CI on five targets | to do |

## Documentation

- [`docs/tutorial/`](docs/tutorial/README.md) — step-by-step travel time and referral
- [`docs/coverage.md`](docs/coverage.md) — AccessMod 5 features: covered, planned, needing a UI
- [`benchmark/`](benchmark/README.md) — performance against AccessMod 5, per module
- [`docs/cost-model.md`](docs/cost-model.md) — cost model, neighbours and direction codes
- [`docs/referral.md`](docs/referral.md) — referral design, decisions and differences from AccessMod 5
- [`docs/plan.md`](docs/plan.md) — architecture, contracts and milestones
- [`docs/ui.md`](docs/ui.md) — web interface: architecture, state rules, deployment, scale, slices
- [`docs/reference-accessmod5.md`](docs/reference-accessmod5.md) — where the
  oracle behaviour lives in the AccessMod 5 repository
- [`AGENTS.md`](AGENTS.md) — workflow, commands, conventions (humans and agents)
- [`STYLE.md`](STYLE.md) — Rust style guide

## License

LGPL-3.0-or-later. See [`LICENSE`](LICENSE) (LGPL-3.0) and [`COPYING`](COPYING) (GPL-3.0, which the LGPL supplements).
