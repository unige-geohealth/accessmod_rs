# accessmod-rs

Portable rewrite of [AccessMod](https://github.com/unige-geohealth/accessmod):
physical accessibility to health services (travel time, nearest facility,
coverage, catchments) as a standalone engine, without GRASS GIS, R or Docker.

This engine is the future reference implementation of AccessMod and is meant to
replace the AccessMod 5 stack (R/Shiny + GRASS + Docker). Until it reaches
feature parity, AccessMod 5 is used **only as a numerical oracle**: its fixtures
and outputs validate this engine; its code is not copied.

> Status: pre-alpha. Nothing is usable yet. See [`docs/plan.md`](docs/plan.md).

## Deliverables (planned)

| Milestone | Deliverable |
| --- | --- |
| 1 | `accessmod` CLI: complete travel-time engine on disk-backed tiled rasters (isotropic/anisotropic, nearest, path distance) |
| 2 | MCP stdio server + agent skill over the same public API |
| 3+ | Zonal stats, coverage, referral, optimization, project import |
| last | Graphical UI on top of the stabilised CLI/MCP contracts |

## Documentation

- [`docs/plan.md`](docs/plan.md) — architecture, contracts and milestones
- [`docs/reference-accessmod5.md`](docs/reference-accessmod5.md) — where the
  oracle behaviour lives in the AccessMod 5 repository
- [`AGENTS.md`](AGENTS.md) — workflow, commands, conventions (humans and agents)
- [`STYLE.md`](STYLE.md) — Rust style guide

## License

LGPL-3.0-or-later. See [`LICENSE`](LICENSE) (LGPL-3.0) and [`COPYING`](COPYING) (GPL-3.0, which the LGPL supplements).
