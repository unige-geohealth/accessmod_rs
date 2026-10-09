# AGENTS.md — accessmod-rs

Guide for humans and coding agents (Claude Code, Codex, …) working in this
repository. Read [`docs/plan.md`](docs/plan.md) first: it is the source of
truth for architecture, contracts and milestones. Code style is in
[`STYLE.md`](STYLE.md).

## Ground rules

- **Language**: everything is in English — code, comments, docs, plans,
  commit messages, issues.
- **Units**: internal values and canonical outputs are SI — **seconds** and
  **metres** (km/h only as scenario input speeds). Minutes, kilometres and
  rounding exist only at the presentation edge: reports, summaries, display,
  compatibility exports.
- **Memory**: never assume the grid fits in RAM. Targets range from small
  machines (2 GB RAM, 2 CPU) to compute servers (40 GB RAM, 12 CPU). All
  raster work goes through the tiled, disk-backed storage with an explicit
  memory budget (see `docs/plan.md`, "Storage and memory").
- **Tooling dependencies**: building, testing and generating artefacts (e.g.
  the bicycle LUT) need only the Rust toolchain. Helper scripts are Rust
  (`cargo xtask …`) or POSIX `sh`. No R; no Python in the build or test path.
  Docker is needed only to export the AccessMod 5 oracle.

## Commits

- Conventional commits: `feat:`, `fix:`, `refactor:`, `perf:`, `test:`,
  `docs:`, `chore:`, `ci:`; optional scope, e.g. `feat(core): …`.
- **One line only. No body. No `Co-Authored-By` or any other trailer.**
- Commit only what the change needs; never commit `_oracle/`, `_am5/`,
  `_shared/` or `target/`.

## Multiple agents

Claude Code and Codex may implement the same task in parallel; the best
implementation is kept.

- Work in a dedicated branch per agent and task, e.g. `claude/<topic>`,
  `codex/<topic>`, branched from `main`.
- Keep each branch self-contained and passing all checks so it can be
  compared and merged as a unit.
- Do not edit another agent's branch.
- Summarise in the final message (or PR description): what was done, what was
  verified, known gaps, and any deviation from `docs/plan.md`.

## Relationship with AccessMod 5

- This repository is independent: it does not vendor, submodule or build
  anything from <https://github.com/unige-geohealth/accessmod>.
- AccessMod 5 is the **numerical oracle**, pinned at tag `5.9.2-beta.6`.
  Where each reference behaviour lives is listed in
  [`docs/reference-accessmod5.md`](docs/reference-accessmod5.md).
- Reimplement from specification and oracle outputs. Do not translate
  AccessMod 5 or GRASS source line by line (GRASS modules are GPL).
- A divergence from AccessMod 5 is allowed only when deliberate, documented in
  `docs/`, and reported in `run.json` warnings when user-visible.

### AccessMod 5 source

A read-only shallow clone of the pinned tag lives in `_am5/accessmod/`
(ignored by git). Create or verify it with:

```sh
sh scripts/fetch-am5.sh
```

- Answer behaviour questions (NoData, neighbours, limits, rounding, LUT, …)
  from this source before asking the user; cite paths relative to
  `_am5/accessmod/`, never absolute or sibling-checkout paths.
- Never edit it, copy code from it or build it; the GPL rule above applies.
- **Vocabulary**: domain names (CLI, job fields, outputs, types, docs) follow
  the English names in `www/dictionary/classes.json` and `main.json`, e.g.
  *facility* (French *structure*), *land cover*, *cost allocation*. "Source"
  is reserved for solver internals.

## Commands

The workspace is not scaffolded yet. Once it is:

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo xtask lut            # regenerate and verify the bicycle LUT
cargo run -p accessmod-cli -- doctor --json
```

Build, test, clippy and fmt must all pass before a commit.

## Layout (target)

```
crates/accessmod-core   grids, tiled storage, mobility models, Dijkstra, domain types (no GDAL)
crates/accessmod-io     GDAL/PROJ boundary, job validation, reports
crates/accessmod-cli    `accessmod` binary (clap)
crates/accessmod-mcp    milestone 2: thin MCP adapter over the public API
xtask/                  developer tasks (LUT generation, oracle export driver, …)
tests/fixtures/         small synthetic fixtures, hand-computable, committed
_oracle/                AccessMod 5 exports (ignored; checksummed manifest committed)
_am5/accessmod/         pinned AccessMod 5 source, read-only (ignored; `scripts/fetch-am5.sh`)
_shared/                local scratch, issue reproductions (ignored)
scripts/                POSIX sh helpers
docs/                   plan, references, design decisions
```

`accessmod-core` never depends on GDAL, PROJ, a project layout or an external
process.

## Local scratch

Use `_shared/<topic>/` for investigations, throwaway scripts and comparison
outputs (ignored by git). Keep outputs small and readable when they may be
pasted into issues.

## Oracle data

AccessMod 5 exports live under `_oracle/` (ignored). Each export is produced
by a committed script run against the pinned AccessMod 5 Docker image and
described by a committed manifest (files, SHA-256, oracle version). Tests that
need `_oracle/` skip with an explicit message when it is absent; they never
pass vacuously.

## Branches and releases

- `main` always builds and passes all checks.
- Short-lived feature branches merged into `main`.
- Releases are tag-driven (`vX.Y.Z`); versioning is independent from
  AccessMod 5.
