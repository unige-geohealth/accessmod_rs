# Style guide

Priorities, in order: **correctness, determinism, auditability**, then
performance. Code is read by people who must trust its numbers.

## Formatting and lints

- Rust 2024 edition, toolchain pinned in `rust-toolchain.toml`.
- `cargo fmt` with the repository `rustfmt.toml`; no manual formatting debates.
- `cargo clippy -- -D warnings` is a gate. Silence a lint only locally, with
  `#[allow(...)]` and a one-line reason.
- `#![forbid(unsafe_code)]` in `accessmod-core`. Elsewhere, `unsafe` requires a
  `// SAFETY:` comment stating the invariant.

## Errors

- Library crates: typed errors with `thiserror`, one enum per crate boundary.
  No `anyhow`, no `unwrap()`/`expect()` on user-controlled data.
- `expect("…")` is acceptable only for internal invariants, with a message
  explaining why it cannot fail.
- The CLI maps errors to exit codes (`0` ok, `2` validation, `3` I/O,
  `4` computation, `130` cancelled). Messages say what failed, on which input,
  and what the user can do.
- Non-test code does not panic on input (files, job fields, command
  parameters, HTTP bodies).
- `accessmod-server` denies `clippy::unwrap_used`, `expect_used`, `panic`,
  `indexing_slicing`, `todo`, `unimplemented` from its first commit
  (allowed in tests); other crates follow when touched.
- Sizes, offsets and counts from input use checked arithmetic and are capped
  before allocating.
- The server catches a panic escaping a command or a run, reports it as an
  error and keeps serving.

## Numerics

- Times and distances are `f64`, in **seconds** and **metres**. Units are part
  of names (`travel_time_s`, `step_length_m`, `speed_kmh`) or of newtypes.
- No silent conversion: reprojection, unit change or resampling are explicit
  operations, logged and recorded in `run.json`.
- Minutes, kilometres and rounding belong to the presentation layer only
  (reports, display, compatibility exports), never to computation.
- Raster access goes through the tiled store; never allocate a full-grid
  buffer (`Vec` of rows × cols) outside it.
- Every comparison that can tie has an explicit, documented tie-break.
  `HashMap` iteration order never reaches an output; use ordered collections
  or sort.
- Floating-point formulas that mirror the AccessMod 5 oracle cite the reference
  location (see `docs/reference-accessmod5.md`) in a comment.
- Constants from scientific sources cite the source (author, year) next to the
  constant.

## API and types

- Domain types live in `accessmod-core`; I/O types never leak into it.
- Public structs that cross the CLI or HTTP boundary derive `serde` and
  `schemars::JsonSchema`. Schemas are versioned (`schema_version`); a breaking
  change bumps the version, never mutates it.
- Prefer newtypes and enums over strings and booleans (`Direction::TowardsFacilities`,
  not `return_path: bool`).
- Keep functions small enough to unit-test with hand-computed values.

## Logging and output

- `tracing` for logs and progress, always on stderr.
- stdout carries only machine-readable output when `--json` is set.

## Tests

- Each behaviour change ships with a test: unit test with hand-computable
  values first, property test where an invariant exists (non-negative cost,
  determinism under input permutation, …), oracle comparison when AccessMod 5
  defines the expected result.
- Tests are deterministic: fixed seeds, no network, no wall-clock assertions.
- Benchmarks (`criterion`) live beside the crate and are not run in CI gates.

## Dependencies

- Prefer the GeoRust ecosystem (`geo`, `geo-types`, `gdal`, `proj`).
- A new dependency needs a reason in the commit message; check license
  compatibility with LGPL-3.0-or-later and avoid unmaintained crates.
- `Cargo.lock` is committed.

## Comments and docs

- Comment *why*, not *what*. Public items have doc comments with units and
  failure modes.
- Design decisions that change behaviour go in `docs/` (short, dated).

## TypeScript (`web/`)

- `strict` on; no `any`, no non-null assertion without a comment saying why.
- Types for jobs, reports and API bodies are generated from the Rust
  schemas (`cargo xtask schemas`); never write them by hand.
- State follows [`docs/ui.md`](docs/ui.md), "State": typed messages, pure
  `update` functions, derived values as selectors, no effect that sets
  state from other state.
- Unions over flags: a run is `{ status: "running"; progress } | { status:
  "done"; report } | …`, handled with an exhaustive `switch`.
- MapLibre and Tabulator are wrapped in adapters that take state and emit
  messages; components never call them directly.
- Biome formats and lints; Vitest covers update functions, selectors and
  state machines without a DOM.
- User-visible text goes through the i18n catalogue (AccessMod 5 keys and
  vocabulary); no literal strings in components.
