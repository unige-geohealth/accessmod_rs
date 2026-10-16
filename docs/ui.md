# Graphical interface — feasibility and architecture

Status: decision draft, 2026-10-16. Implements `plan.md`, Later 5.

## Goal

Users of AccessMod 5 must find their way in the new interface: the same
sections, the same vocabulary, the same workflow. The goal is not a copy of
the screens. The AccessMod 5 interface is a reactive R/Shiny application
written about ten years ago, and it breaks easily: when one input changes,
long chains of reactive code run again.

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

## Feasibility

The technical feasibility is high, about 85–90 %. None of these widgets is
hard to build. The risk lies in the scope.

Rough effort for one developer assisted by agents, once the engine contracts
are stable:

| Stage | Content | Effort |
| --- | --- | --- |
| MVP | Project, data catalogue, Accessibility, Referral, map preview, logs, four languages | 2–4 months |
| Feature parity | Adds coverage, zonal statistics (charts), scaling up, interactive merge land cover, facility relocation, population correction | +4–6 months |

The engine for the missing analyses is a prerequisite and is not counted in
these figures. It sets the calendar.

## Decision

### Web front end served by the binary

- `accessmod serve` (axum) exposes a JSON HTTP API and serves a static
  TypeScript front end.
- Tauri can wrap the same front end as an optional desktop package. Server
  deployment, as AccessMod 5 has today, keeps working with the same code.
- Maps: MapLibre GL (or Leaflet). Tables: Tabulator, which AccessMod 5
  already uses, so editing, sorting and filtering behave the same; TanStack
  Table is the alternative.
- No heavy reactive framework. The server holds the state. The client holds
  only view state and the edits in progress.

Rejected:

- **Slint, egui, iced**: good for native forms, but they have no mature
  slippy map and no data grid at this level. We would have to build both
  ourselves.
- **TUI**: the interactive tasks are spatial (merge stack, artefact
  correction, facility relocation). A terminal serves them poorly, and it
  would add a third interface to maintain.

### Server and client responsibilities

- **The engine stays CLI-first.**
  - Every interface action produces a `job.json`.
  - The job can be replayed and validated with the CLI.
  - The interface computes nothing.
- **Project and catalogue layer** (`plan.md`, Later 4): datasets, tags and
  run history. The CLI, MCP and the interface all share it.
- **HTTP API**:
  - It is built on the same functions as the MCP tools: inspect, schema,
    validate, run, report, progress and cancellation.
  - `schemars` schemas are served to the client for form validation, so the
    rules exist in one place only.
- **Client state**: the parts that only a client can handle.
  - Table editing and sorting: scenarios, facility selection, capacities.
  - Ordering the merge stack by drag and drop, and reviewing conflicts.
  - Map selection and point relocation.
  - Zonal charts.
  - None of this is business logic: it is editing state, submitted as a job.
- **Languages**:
  - `main.json` and `classes.json` are converted into an i18n catalogue
    served to the client.
  - Keys and vocabulary follow the AccessMod 5 dictionary.

### MCP before the interface

The MCP server (milestone 2) already covers part of the "interactive" need.
An agent can:

- build a merge stack;
- explain class conflicts;
- propose facility relocations as a job.

It also forces clean contracts, which the interface then reuses.

## Screens and jobs

| AccessMod 5 screen | Job or API |
| --- | --- |
| Project (open, new, delete, export) | project layer (Later 4) |
| Data (catalogue, tags, import) | project layer, `dataset inspect` |
| Analysis › Accessibility | `travel-time run` |
| Analysis › Referral | `referral run` |
| Analysis › Geographic coverage, Zonal statistics | Later 1 jobs |
| Analysis › Scaling up, Optimization | Later 3 jobs |
| Toolbox › Merge land cover | Later 6 merge job (stack as an ordered list) |
| Toolbox › Adjust population distribution | Later 6 population job |
| Toolbox › Interactive map | client only; relocations become an edited facility layer |
| Logs | `run.json` history |
| Settings › Language | client i18n catalogue |

## Sequence

1. Engine coverage through the CLI and jobs: merge land cover with a
   conflicts report, population correction, coverage, zonal statistics,
   then scaling up and optimization.
2. Project format and catalogue (Later 4); MCP (milestone 2).
3. Early **throwaway prototype**: `accessmod serve`, MapLibre and Tabulator
   on Accessibility. It validates the architecture and the user experience
   without waiting for step 1 to finish.
4. Interface MVP, then parity module by module.
