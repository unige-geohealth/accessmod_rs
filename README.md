# AccessMod 6

[![ci](https://github.com/unige-geohealth/accessmod_rs/actions/workflows/ci.yml/badge.svg)](https://github.com/unige-geohealth/accessmod_rs/actions/workflows/ci.yml)
[![release](https://github.com/unige-geohealth/accessmod_rs/actions/workflows/release.yml/badge.svg)](https://github.com/unige-geohealth/accessmod_rs/actions/workflows/release.yml)
[![version](https://img.shields.io/badge/version-6.0.0--alpha.1-orange)](docs/coverage.md)
[![licence](https://img.shields.io/badge/licence-LGPL--3.0--or--later-blue)](LICENSE)
[![rust](https://img.shields.io/badge/rust-1.92-brown)](rust-toolchain.toml)

Physical accessibility to health services: travel time to facilities,
geographic coverage and catchments, referral, zonal statistics, scaling up
and optimization. AccessMod 6 is the rewrite of
[AccessMod 5](https://github.com/unige-geohealth/accessmod) (R/Shiny, GRASS
GIS, Docker) as one binary with a web interface, in Rust (repository
`accessmod-rs`).

**Status: alpha.** Every AccessMod 5 analysis runs from the command line
and gives the same results as AccessMod 5 on its demo project; the web
interface runs them all. Toolbox screens, the interactive map and `.am5p`
import are next. What is done, feature by feature:
[`docs/coverage.md`](docs/coverage.md).

AccessMod 5 serves only as the numerical oracle: its outputs validate this
engine in the tests; its code is not copied.

## Install

Release archives (binary with GDAL, PROJ and the web client, nothing else
to install) come from the [releases](https://github.com/unige-geohealth/accessmod_rs/releases);
Linux (x86_64, arm64), macOS (Apple silicon, Intel) and Windows (x86_64)
([`docs/distribution.md`](docs/distribution.md)).

```sh
./accessmod doctor        # check GDAL, PROJ, drivers, memory and disk
./accessmod serve         # web interface, http://127.0.0.1:5099 (projects in ~/AccessMod)
./accessmod project demo demo   # the AccessMod 5 sample project
```

From source (needs GDAL ≥ 3.5 and PROJ, or `--features bundled` to build
them):

```sh
cargo build --release -p accessmod-cli
./target/release/accessmod travel-time run --job docs/tutorial/data/job.json
```

The [tutorial](docs/tutorial/README.md) walks through inputs, jobs, outputs
and figures on a small synthetic landscape.

## Deliverables

| Milestone | Deliverable | Status |
| --- | --- | --- |
| 1 | Travel-time engine on disk-backed tiled rasters, under a memory budget | ✅ |
| 2 | Every analysis and tool as a command-line job, equal to AccessMod 5 | ✅ (referral network output 🗓) |
| 3 | Projects and web interface (`accessmod serve`) | 🟡 analyses, data and projects ✅; toolbox, map, logs 🗓 |
| 4 | Distribution: self-contained archives, CI | 🟡 CI on every push (Linux, macOS with bundled GDAL, web and end-to-end); release archives for Linux, macOS and Windows ✅; macOS notarization in CI 🗓 |

## AccessMod 6 against AccessMod 5

Median of 3 runs on an Apple M1 Pro, AccessMod 5 in its Docker image
([modules, scripts and reports](benchmark/README.md)). Demo project, full
analysis (AccessMod 5 replay without its 5.4 s of R start-up):

| Module | AccessMod 5 | AccessMod 6 |
| --- | ---: | ---: |
| Travel time | 1.42 s | 0.24 s |
| Referral, 90 pairs | 11.67 s | 0.23 s |
| Geographic coverage | 14.55 s | 0.40 s |
| Scaling up, 10 facilities added | 39.39 s | 0.50 s |
| Optimization, 10 facilities, 1 per zone | 4.24 s | 0.29 s |

16 M-cell synthetic grids (AccessMod 5: its engine alone, a lower bound):

| Module | AccessMod 5 | AccessMod 6 | Memory AM5 → AM6 |
| --- | ---: | ---: | ---: |
| Travel time, 10 facilities | 15.8 s | 16.1 s (solve 14.7 s) | 2.1 → 0.7 GB |
| Referral, 9 900 pairs | ≥ 35.7 s | 13.5 s | — → 0.7 GB |
| Geographic coverage, 100 facilities | ≥ 133.9 s | 19.7 s | — → 0.5 GB |

- **Same numbers**: travel times agree with AccessMod 5 to 1e-15 (relative)
  on its demo project; every analysis is checked against AccessMod 5
  outputs, and deliberate differences are documented
  ([coverage](docs/coverage.md)).
- **Bounded memory**: a hard budget (`--memory`) with disk-backed tiles;
  results are byte-identical whatever the budget, so national grids run on
  small machines.
- **Faster analyses**: analyses built on many searches gain most, as only
  reached cells are visited; the travel-time solver is on par with
  `r.walk.accessmod`.
- **One binary, scriptable**: no R, GRASS or Docker; job files, JSON output
  and exit codes; every action of the web interface is a reproducible job.

## Documentation

- **Use**: [tutorial](docs/tutorial/README.md) ·
  [coverage of AccessMod 5 features](docs/coverage.md) ·
  [benchmarks](benchmark/README.md) ·
  [distribution](docs/distribution.md)
- **Analyses**: [cost model](docs/cost-model.md) ·
  [referral](docs/referral.md) ·
  [geographic coverage](docs/geographic-coverage.md) ·
  [zonal statistics](docs/zonal-statistics.md) ·
  [scaling up](docs/scaling-up.md) ·
  [optimization](docs/optimization.md) ·
  [merge land cover](docs/merge-land-cover.md) ·
  [adjust population](docs/adjust-population.md)
- **Design**: [plan: architecture, contracts, milestones](docs/plan.md) ·
  [web interface](docs/ui.md) ·
  [design system](design_system/README.md) ·
  [where AccessMod 5 behaviour lives](docs/reference-accessmod5.md)
- **Contribute**: [workflow and commands](AGENTS.md) ·
  [style](STYLE.md)

## Development

The Rust workspace needs the Rust toolchain (pinned) and GDAL, PROJ; the
front end needs Node 24 and npm.

```sh
sh scripts/check.sh   # Rust gate: fmt, clippy, tests, layers, LUT, schemas, i18n
npm ci                # front-end dependencies (design system and client)
npm run dev           # API (debug build, port 5099) + client, http://localhost:5173
npm run check         # front-end gate: typecheck, contrast, tests, builds
npm run e2e           # Playwright end-to-end tests (after `npm run build`)
sh scripts/package.sh # release archive in dist/ (bundled GDAL)
sh scripts/release.sh 6.0.0-alpha.2 --dry-run   # from main: gates, then version, tag, push
```

Every command, the conventions and the pitfalls: [`AGENTS.md`](AGENTS.md).

## Licence

LGPL-3.0-or-later: [`LICENSE`](LICENSE) (LGPL-3.0) and
[`COPYING`](COPYING) (GPL-3.0, which the LGPL supplements). Release archives
list the libraries they contain in `THIRD-PARTY-LICENSES.md`.
