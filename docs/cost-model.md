# Cost model `accessmod5-v1`

Implemented in `crates/accessmod-core/src/cost.rs` and `src/mobility/`.
Oracle references are listed in `reference-accessmod5.md`.

## Steps and direction codes

Rows grow southwards. Codes follow the AccessMod 5 neighbour order
(`r.walk.accessmod/main.c`, cases 1–16) and are the values written to
`direction.tif` (0 = no predecessor: facility cell or unreached).

| Code | Step | (Δrow, Δcol) | Kind | Crossed cells (summation order) |
| ---: | --- | --- | --- | --- |
| 1 | W | (0, −1) | cardinal | |
| 2 | E | (0, +1) | cardinal | |
| 3 | N | (−1, 0) | cardinal | |
| 4 | S | (+1, 0) | cardinal | |
| 5 | NW | (−1, −1) | diagonal | |
| 6 | NE | (−1, +1) | diagonal | |
| 7 | SE | (+1, +1) | diagonal | |
| 8 | SW | (+1, −1) | diagonal | |
| 9 | NNW | (−2, −1) | knight | N, NW |
| 10 | NNE | (−2, +1) | knight | N, NE |
| 11 | SSE | (+2, +1) | knight | S, SE |
| 12 | SSW | (+2, −1) | knight | S, SW |
| 13 | WNW | (−1, −2) | knight | W, NW |
| 14 | ENE | (−1, +2) | knight | E, NE |
| 15 | ESE | (+1, +2) | knight | E, SE |
| 16 | WSW | (+1, −2) | knight | W, SW |

The code is the step from the settled (origin) cell to the cell it reached,
in the solver's expansion direction (always outwards from the facilities).

Step lengths: `ew`, `ns`, `√(ns² + ew²)`, `√(4ns² + ew²)` (two rows),
`√(ns² + 4ew²)` (two columns).

## Edge cost

For an edge of planar length `L` (m), cells `cᵢ` with speed `vᵢ` (km/h):

```text
adjacent: cost_h = 1/(v_origin/(L/2000)) + 1/(v_target/(L/2000))
knight:   cost_h = (1/(v_o/(L/4000)) + 1/(v_straight/(L/4000)))
                 + (1/(v_diagonal/(L/4000)) + 1/(v_target/(L/4000)))
cost_s  = cost_h · 3600
```

This is the AccessMod 5 operation order, kept so that identical inputs give
bit-identical edge costs. A non-finite cost (zero speed) means no edge.

- **Anisotropic**: `vᵢ = model(modeᵢ, flat speedᵢ, slope)` with one slope
  `Δz / L` per edge (`Δz` = target − origin), negated for
  `towards_facilities`.
- **Isotropic**: `vᵢ` = scenario flat speed; mode and slope ignored.
- **Barriers**: NoData land cover, class without scenario, zero speed,
  NoData DEM. A knight step is forbidden when any of its four cells is a
  barrier.

## Bicycle LUT

`cargo xtask lut` samples the model on 201 flat speeds (0–100 km/h, step 0.5)
× 441 slopes (−1.2–1.0, step 0.005), rounds each value as the AccessMod 5 C
header does (`%.8f`, then `float`) and writes
`crates/accessmod-core/data/bicycle-lut-accessmod5-v1.f32le` (little-endian
`f32`, row-major). Run time interpolates it bilinearly in `f64` after
clamping. The committed table is bit-identical to the oracle's.
