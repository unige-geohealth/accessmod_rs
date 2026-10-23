# Graphical interface

Status: decision, 2026-10-23. Implements `plan.md`, milestone 3. Replaces
the 2026-10-16 draft, which kept the interface after MCP.

## Goal

Users of AccessMod 5 must find their way in the new interface: the same
sections, the same vocabulary, the same workflow. The goal is not a copy of
the screens.

Every analysis already runs from the CLI and matches the AccessMod 5
oracle. The interface is now the shortest path to user feedback, so it comes
before MCP (deferred, see the end of this file).

The same server runs two ways:

- **desktop**: on the user's machine, bound to `127.0.0.1`, one implicit
  user;
- **country server**: deployed by a country, e.g. on a small server of the
  ministry of health, used from browsers on its network by several
  authenticated users.

## What AccessMod 5 has

Reference: `5.9.2-beta.6`; paths are relative to `_am5/accessmod/`.

- **Navigation**: a sidebar with Project, Data, Analysis, Toolbox, Logs,
  Settings and About.
  - Analysis tabs: Accessibility, Geographic coverage, Referral, Zonal
    statistics, Scaling up, Optimization.
  - Toolbox tabs: Interactive map, Merge land cover, Adjust population
    distribution.
- **Widgets**: forms, editable tables (`amtabulator`, which wraps
  Tabulator.js), a Leaflet map, progress bars and modals.
- **Size**:
  - about 11,500 lines of R UI and server code in `modules/`, of which
    4,756 are Accessibility and 2,030 are Merge land cover;
  - 53 files in `tools/R`;
  - about 1,200 lines of custom JavaScript (`www/modules/accessmod`).
- **Languages**: `www/dictionary/main.json` has 702 keys in en, fr, es and
  pt (de is empty). `classes.json` has the data classes.

The widgets themselves are simple. The complexity comes from the reactive
wiring between them.

### What to avoid

All interface state lives in Shiny reactives on the R side. Each input is a
node of a reactive graph, and the graph is never written down: a value read
inside a reactive becomes a dependency, and observers update other inputs,
which wake further observers. The result is fast and optimised, but one
change runs long, invisible chains, and order, re-entrancy and partial
updates are hard to reason about or to change safely.

The interface must make every dependency explicit. This is the main design
constraint, ahead of any library choice.

## Principles

1. **CLI-first.** Every action that computes produces a `job.json` that the
   CLI replays. The interface computes nothing scientific.
2. **Every action is a command** (see "Commands"): the HTTP API, the CLI and
   a future MCP dispatch the same registry.
3. **One owner per piece of state**, and one way to change it (see "State").
4. **No cascades.** State changes only in response to a user message or a
   server event, never in response to another state change.
5. **Derived values are computed, never stored.**
6. **Types are defined once, in Rust**, and generated for TypeScript.
7. **Never crash.** Bad input, a corrupt file or a full disk gives an error
   the user can act on; the server keeps running.
8. **Built for national scale**: 400 M-cell rasters, every hospital or school
   of a country (see "Scale").
9. **Testable in thin slices.** Each slice ends with something a user can
   open and try.

## Architecture

```text
browser (TypeScript)                         accessmod serve (Rust)
  view (React) ── message ──▶ update           axum: JSON API + SSE events
  store ◀── response / event ── API client ◀─▶ command registry (auth, roles)
  MapLibre, Tabulator (adapters)                ├─ project layer (accessmod-io)
                                                └─ runner ─▶ Analysis::prepare / run
```

### Server: `accessmod-server`

- New crate, axum on tokio. Command:
  `accessmod serve [--data <dir>] [--host 127.0.0.1] [--port <n>] [--open]`.
- **Generic over `Analysis`**: listing, schemas, validation and runs work for
  every analysis; registering one is a single line. No analysis-specific
  endpoint until a bespoke screen needs one.
- **API** (resource-oriented JSON over the command registry):
  - `GET /api/doctor`, `GET /api/commands` (ids, parameter schemas, roles);
  - session: sign in and out, current user;
  - projects: list, create, open, delete, export (archive);
  - datasets: list (by class and tags, paged), import (resumable upload or,
    on desktop, a local path), inspect, tag, delete; raster tiles
    `GET /api/datasets/{id}/tiles/{z}/{x}/{y}.png`; vector tiles
    `GET /api/datasets/{id}/tiles/{z}/{x}/{y}.mvt`; attribute rows (paged,
    sorted, filtered on the server); values at a point;
  - analyses: `GET /api/analyses` (names, JSON Schemas),
    `POST /api/analyses/{name}/validate`;
  - jobs: saved drafts `GET|PUT|DELETE /api/jobs/{id}`;
  - runs: `POST /api/runs` (a job), `GET /api/runs`, `GET /api/runs/{id}`
    (`run.json`), `DELETE /api/runs/{id}` (cancel), result tables (paged);
  - `GET /api/events`: server-sent events for run progress and state, and
    catalogue changes, filtered to what the user may see.
- **Runner**: a queue with admission control (see "Deployment"). A run calls
  the same `prepare` / `run` as the CLI, on a blocking thread, with the
  cancellation flag and a progress sink. Outputs are byte-identical to the
  CLI's. Runs survive a closed browser; after a server restart, unfinished
  runs are marked interrupted, never left running.
- **Static assets**: the built front end is embedded with `rust-embed` behind
  the cargo feature `web` (release builds). Without it, `serve` takes
  `--web-dir`, or the Vite dev server proxies `/api`. `cargo build
  --workspace` never runs Node.

### Commands

Every action that changes something (create a project, import, tag, save a
draft, run, cancel, manage users) is a command in one registry:

- an identifier (`dataset.import`, `run.start`), a parameter type deriving
  `JsonSchema`, the role it requires, and a function returning `Result`;
- the HTTP API dispatches commands by identifier; the CLI calls the same
  functions; a future MCP server is a thin listing of the same registry;
- every command validates its parameters and the project state, and returns
  an error, never panics: a test sends adversarial parameters to every
  command (wrong types, out of range, missing datasets, foreign projects);
- reads (listings, tiles, tables) are plain queries, checked against the
  same roles.

### Engine contracts the interface needs

- **Progress**: `Analysis` receives a progress sink (stage, done, total) next
  to the cancellation flag. The CLI prints it on stderr, the server emits it
  as events. It never changes outputs.
- **Dataset classes on job fields**: path fields carry the expected AccessMod
  5 class (`x-accessmod-class` in the JSON Schema, e.g. `rLandCover`,
  `vFacility`), so a form offers only matching datasets and validation checks
  the class.
- **Preview data** (`accessmod_io::preview`): when a raster enters a
  project (import or run output), its overviews go to an external
  `<raster>.ovr`, which GDAL reads with the raster, and its statistics
  (range, mean, histogram, distinct classes) to the catalogue. The raster
  itself is never rewritten, so canonical outputs keep their bytes and
  analysis timings are unchanged; previews and legends never scan a whole
  grid.
- **Cheap checks**: `validate` runs `prepare`, which may read whole inputs.
  Measure it on a national grid; if it is too slow for checks while editing,
  split a metadata-only `check` (CRS, lattice, fields, classes) from
  `prepare`.

### Project layer

`plan.md`, milestone 3. Shared by the CLI, the server and a future MCP.
Metadata in SQLite, data in files:

```text
<data>/
  server.sqlite           users, roles, sessions, run queue (country server)
  projects/<id>/
    project.sqlite        project, grid (from the DEM), datasets (class, tags,
                          origin, fingerprint, statistics), saved jobs, runs
    datasets/<id>.tif     rasters: tiled GeoTIFF (+ .ovr overviews)
    datasets/<id>.gpkg    vectors being edited (facility relocation, selections)
    datasets/<id>.fgb     vectors fixed once written (run outputs)
    runs/<id>/            outputs, job.resolved.json, run.json
    cache/                preview tiles (disposable)
```

- **SQLite** (bundled with the binary, WAL mode): several users write at once
  on a country server; no database service to administer; a backup is a
  copy of the directory.
- **Data stays in files.** No raster in a database. Vectors that users edit
  are GeoPackage (SQLite with a spatial index: paged, sorted and filtered
  reads, edits in place); canonical run outputs stay FlatGeobuf, since a
  GeoPackage embeds timestamps and breaks byte identity.
- **Jobs reference datasets of the project only**, never an arbitrary path,
  on desktop as on a server. A job exported from a project is a plain
  `job.json` that `accessmod <analysis> run` replays.
- Classes and tags follow `classes.json` and the AccessMod 5 tag rules.
- Run outputs are registered as datasets, as in AccessMod 5 (a travel time
  becomes an input of zonal statistics).
- **Import**: a raster on the project lattice is converted to tiled GeoTIFF
  with `.ovr` overviews; a misaligned raster needs an explicit resampling step
  whose method is recorded, never an implicit one. Vectors are converted to
  the project CRS.
- CLI (done): `accessmod project init|info|run|runs|recover`, `accessmod
  dataset import|list|tag|remove`. A run in a project gives the same bytes as
  the plain CLI on the same inputs (checked on the tutorial). Tags follow
  AccessMod 5: any character but letters and digits separates them, so
  `travel_time` is two tags; with its class, a tag set names one dataset.
- `.am5p` import: a separate converter, after the first testable slice.

### Client: `web/`

- TypeScript (`strict`), Vite, React 19, MapLibre GL, Tabulator, TanStack
  Query.
- Node LTS pinned (`web/.nvmrc`, `engines`), npm with a committed lockfile,
  Biome (lint and format), Vitest, Playwright.

## State

Four kinds of state, each with one owner and one way to change:

| Kind | Examples | Owner | Changed by |
| --- | --- | --- | --- |
| Server | projects, catalogue, dataset metadata, runs, reports | server | API responses and events, held in the query cache |
| Document | the job being edited, merge stack, scenario table, facility selection | client store, one per document | typed messages through a pure `update` |
| Process | run: queued → running (progress) → done, failed, cancelled or interrupted; import | server, mirrored | events, into an explicit state machine |
| View | route, open panel, map viewport, table sort | component or URL | local |

Rules:

- **A document is a typed job draft** (generated types). Saving is `PUT`,
  running is `POST` of the same document: what the user sees is what the
  CLI replays.
- **`update(state, message) → state` is pure.** Side effects (save, validate,
  run, import) start from a message handler, never from observing state.
- **No effect writes state from other state.** `useEffect` only syncs an
  imperative widget from state or subscribes to events. Reviews treat a
  state-setting effect as a defect.
- **Derived values are selectors**: pure, memoised, never stored (classes of
  the land cover missing from the scenario, facility counts, form errors).
- **Server checks are queries keyed by a hash of the draft**: a newer draft
  aborts the older request, and a result never writes back into the draft.
- **Imperative widgets are adapters**: MapLibre and Tabulator get state and
  emit messages; they hold no domain state.
- **Process state is a discriminated union** with an exhaustive `switch`;
  impossible states do not type-check.
- **Undo and redo** come from the document store: states are immutable, so
  history is a list of states.
- **Drafts persist on the server** (debounced `PUT`): a reload or a crash
  loses nothing.
- **State is serialisable and drivable**: in development builds, a hook
  reads the stores and sends messages, so tests and agents drive the
  interface and take screenshots without clicking through it.

### Why React, and not signals

- Signal frameworks (Solid, Svelte 5, Vue, Preact signals) track
  dependencies automatically: reading a value inside a computation adds an
  edge to a graph nobody wrote down. It is Shiny's model, made synchronous
  and glitch-free, but cascades stay implicit. Render speed is not our
  problem; implicit chains are.
- React renders `view = f(state)` from an explicit store. With the rules
  above, each dependency is visible in code: message → update → state →
  view.
- Map and table wrappers are mature, and React is widely known by
  contributors and coding agents.
- Elm would enforce this architecture through the language, at the cost of
  another language and a small ecosystem. We take its architecture, in
  TypeScript.
- Store: a small typed store over `useSyncExternalStore` (or Zustand with a
  reducer). No Redux Toolkit. No XState at first: machines are discriminated
  unions; reconsider if workflows grow.

## Types and validation

- Rust is the single source: jobs, reports, catalogue entries, command
  parameters, API bodies and events derive `serde` and
  `schemars::JsonSchema`.
- `cargo xtask schemas [--check]` writes the JSON Schemas to `web/schemas/`
  (committed; CI checks they are current, as for the LUT).
- The web build generates TypeScript types from them
  (`json-schema-to-typescript`) and compiles validators (Ajv) for instant
  form checks. The server validates everything again.
- Two levels: the client checks the schema while typing; the server checks
  the inputs (CRS, lattice, fields, classes) and reports every issue, as
  `validate --json` does.

## Scale

National data is the design case, not an edge case: rasters of 400 M cells
(about 20,000 × 20,000), every hospital or school of a country, national
road networks, referral tables of thousands × thousands of facilities. The
engine already handles them under a memory budget; the interface must too.

| Case | Problem | Answer |
| --- | --- | --- |
| 400 M-cell raster preview | a tile at low zoom would read the whole grid | `.ovr` overviews built on entry |
| Colour ramps, legends | a histogram on the fly takes minutes | statistics stored with the dataset |
| 100,000 points, national roads | GeoJSON weighs hundreds of MB | vector tiles from the spatial index |
| Attribute table of 100,000 rows | too heavy to send whole | paged, sorted and filtered on the server; Tabulator in remote mode |
| Selecting 50,000 facilities | an identifier list in `job.json` is huge and fragile | a saved selection (dataset) or an attribute filter |
| Referral table of 5,000 × 5,000 | 25 M rows | CSV or Parquet output; paged rows and aggregates from the server |
| Multi-GB upload | a dropped connection restarts it | resumable, chunked upload |

A national fixture (a 400 M-cell grid, about 100,000 points) joins the demo
project in the interface tests, with time budgets for opening a project,
first preview tile, a table page and a value at a point.

## Deployment

### Desktop

`accessmod serve --open` binds `127.0.0.1`, without sign-in: one implicit
user with every role. Optional Tauri package later.

### Country server

One binary and one data directory on a small server, run as a service
(systemd, or a container):

- **Accounts**: built-in local accounts by default, since a ministry often
  has no identity provider: an administrator creates users; passwords are
  hashed with Argon2. OpenID Connect and a trusted header from a reverse
  proxy are options.
- **Roles per project** (viewer, editor, owner) and server administrator,
  checked by the command registry on every command and query.
- **Every request carries a user** from the first slice, the implicit local
  user on desktop, so adding sign-in later does not touch the handlers.
- **Admission control**: the run queue starts a run only when the sum of the
  running budgets fits the server memory; quotas per user (concurrent runs,
  disk).
- **HTTPS** through a reverse proxy (Caddy, nginx), or served directly with
  `rustls` and a certificate supplied by the country.
- **Operations**: backup by copying the data directory; update by replacing
  the binary; SQLite migrations versioned and applied at start.
- **Hardening**: uploads and paths confined to the data directory, size
  limits on every body, no shell or external process.

## Map preview

- MapLibre GL in Web Mercator. The server renders raster tiles on demand
  from the overviews (GDAL warp per tile) with a ramp per class: land cover
  colours from its labels, continuous ramps from the stored statistics for
  travel time and population. Tiles are cached under `cache/`.
- Vectors as vector tiles (MVT) built from the spatial index; GeoJSON only
  for small layers.
- No basemap by default (offline use); an online basemap is a setting.
- A click shows the values at that point.

## Testing

- **Rust**: API tests in process on the demo project: every command and
  query, roles, the event sequence of a run, cancellation, interrupted
  runs, outputs equal to the CLI's; adversarial parameters for every
  command.
- **TypeScript**: `tsc --noEmit`, Biome, Vitest on update functions,
  selectors and state machines (no DOM needed, since they are pure).
- **End to end**: Playwright on the demo project: open a project, import a
  dataset, run an analysis, see the result and its report; one scenario per
  analysis screen. Screenshots are checked by eye for interface changes.
- **Scale**: the national fixture, against the time budgets above.
- **Parity**: a generated checklist of AccessMod 5 screens and controls
  (modules and `main.json` keys), each live or missing, tracks the
  interface.

## Languages

- `main.json` and `classes.json` are converted by an xtask into an i18n
  catalogue served to the client.
- Keys and vocabulary follow the AccessMod 5 dictionary.

## Screens and jobs

| AccessMod 5 screen | Job or API |
| --- | --- |
| Project (open, new, delete, export) | project commands |
| Data (catalogue, tags, import) | dataset commands, `dataset inspect` |
| Analysis › Accessibility | `travel-time run` |
| Analysis › Referral | `referral run` |
| Analysis › Geographic coverage | `geographic-coverage run` |
| Analysis › Zonal statistics | `zonal-statistics run` |
| Analysis › Scaling up | `scaling-up run` |
| Analysis › Optimization | `optimization run` |
| Toolbox › Merge land cover | `merge-land-cover run` (stack as an ordered list) |
| Toolbox › Adjust population distribution | `adjust-population-distribution run` |
| Toolbox › Interactive map | relocations edit a facility dataset (GeoPackage) |
| Logs | run history (`run.json`) |
| Settings › Language | client i18n catalogue |

## Slices

Each slice passes every check and is usable on its own.

1. **Contracts**: progress sink in `Analysis`, dataset classes on job fields,
   preview data (`.ovr` overviews, statistics), `cargo xtask schemas`,
   `cargo xtask layers`.
2. **Project layer** (SQLite metadata, datasets, saved jobs, runs) and its
   CLI; run outputs registered as datasets.
3. **Server**: `accessmod-server`, `serve`, command registry with the
   implicit local user, generic API, runner, events, tiles; API tests.
4. **Web shell, first testable slice**: AccessMod 5 navigation, project open
   and new, catalogue with import and map preview, a form generated from the
   JSON Schema for every analysis and tool, runs with progress and
   cancellation, run history. Playwright on the demo project.
5. **Country server**: accounts, roles, admission control, quotas,
   resumable uploads; national fixture and time budgets.
6. **Bespoke screens**, ordered by user feedback: Accessibility scenario
   table, merge land cover stack, facility selection, coverage and zonal
   results, facility relocation.
7. **Reach**: four languages, `.am5p` import, self-contained archives with
   GDAL/PROJ (`plan.md`, milestone 4), optional Tauri package.

## Rejected

- **egui (Rust compiled to WebAssembly for the web)**: one language and no
  reactive graph (immediate mode), and it can carry a large application;
  but no map at the level of MapLibre, no data grid at the level of
  Tabulator, and in a browser everything is drawn in a canvas (no text
  selection, browser zoom or accessibility, heavy first load). Computation
  stays on the server anyway: GDAL/PROJ and national grids do not fit in a
  browser tab.
- **Slint, iced**: same limits for maps and tables.
- **TUI**: the interactive tasks are spatial (merge stack, conflict review,
  facility relocation); a terminal serves them poorly.
- **Signal-based frameworks**: see "Why React, and not signals".
- **Server-held view state** (the Shiny model): the server owns data and
  runs, not what a screen shows.
- **Metadata as plain JSON files**: fragile with several users writing at
  once.

## MCP

Deferred: early feedback on an agent interface was lukewarm. If it comes
back, it is a thin `rmcp` adapter over the command registry (`plan.md`,
"Deferred").
