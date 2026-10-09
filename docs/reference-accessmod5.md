# AccessMod 5 — oracle reference

Where the behaviour this engine must reproduce (or deliberately diverge from)
lives in the official repository. Read these files to understand behaviour;
do not translate them line by line.

- Repository: <https://github.com/unige-geohealth/accessmod>
- Pinned oracle: tag **`5.9.2-beta.6`**, commit
  `e845cc96d7ba412665f02833f5d0c75ae305fb87` (branch `staging`)
- Local read-only clone: `_am5/accessmod/`, created by
  `scripts/fetch-am5.sh`; all paths below are relative to it

Permalink pattern:
`https://github.com/unige-geohealth/accessmod/blob/5.9.2-beta.6/<path>#L<a>-L<b>`

Changing the pinned oracle means re-exporting `_oracle/`, updating its manifest
and recording the reason in `docs/`.

## Travel-time pipeline

| Concern | Path | Notes |
| --- | --- | --- |
| Analysis entry point | `tools/R/amAnalysisTravelTime.R` (`amTravelTimeAnalysis`, l.30) | Dispatches isotropic / anisotropic |
| Isotropic | same file, `amIsotropicTravelTime` (l.199), `r.cost` call l.394 | Uses GRASS `r.cost`, **not** the mobility kernel |
| Anisotropic | same file, `amAnisotropicTravelTime` (l.431), `r.walk.accessmod` call l.651 | Uses the mobility kernel |
| Output post-processing | same file, `amCleanTravelTime` (l.692) | Seconds → minutes with `ceil` by default, limit cut, int16/int32 caps. Stored AM5 outputs are **rounded minutes**: the oracle must export raw seconds before this step |
| Isotropic friction | `tools/R/amFrictionSpeed.R`, `amCreateFrictionMap` (l.85) | Friction = seconds per cell = `resolution / (speed/3.6)`, written with `r.recode`. Output cell type to verify (CELL/FCELL/DCELL) |
| Anisotropic speed map | same file, `amCreateSpeedMap` (l.21) | Mode + speed encoded in one value (`amEncodeModeSpeed`) |
| Facility subset | `tools/R/amAnalysisTravelTime.R`, `amFacilitiesSubset` (l.765) | Which facilities enter the computation |
| Memory estimate | same file, `amGetRessourceEstimate` (l.810) | Basis for comparison with our RAM budget |

## Mobility kernel (anisotropic)

| Concern | Path |
| --- | --- |
| Shared kernel | `docker/alpine_base/modules/accessmod/mobility.h` |
| Edge cost | `mobility.h`, `am_cost_seconds` (l.130-176) |
| Walking (Tobler) | `mobility.h`, `am_walking_speed` (l.47) |
| Bicycling (LUT, bilinear) | `mobility.h`, `am_bicycle_speed` (l.54-108) |
| Motorized | `mobility.h`, `am_motorized_speed` (l.110) |
| Bicycle LUT generator | `docker/alpine_base/modules/accessmod/generate-bicycle-lut.R` → `bicycle-lut.generated.h` + `bicycle-lut.csv`, run by `docker/alpine_base/Dockerfile` l.191 (the `-new.R` variant is not used) |
| Bicycle model notes | `docker/alpine_base/modules/accessmod/bicycle-lut.md` |
| Dijkstra / neighbours | `docker/alpine_base/modules/r.walk.accessmod/main.c` (neighbour switch ~l.1240-1790) |
| Step lengths | `main.c` l.402-408 (`EW_fac`, `NS_fac`, `DIAG_fac`, `V_DIAG_fac`, `H_DIAG_fac`) |
| Kernel unit tests | `docker/alpine_base/modules/r.walk.accessmod/tests/` |

Facts established from the code (to be confirmed by oracle runs):

- **Edge cost averages pace, not speed.** Cardinal/diagonal:
  `(1/v_from + 1/v_to) · L/2`; knight: mean pace of 4 cells (from, to and the
  two cells crossed) `· L/4`, in hours × 3600. Equivalent to a harmonic mean of
  speeds.
- **Knight crossed cells**: the two cells one step from the origin along the
  long axis, on both columns (or rows) spanned by the move. For a move of
  (Δrow, Δcol) = (−2, −1) (NNW) they are N (−1, 0) and NW (−1, −1); for
  (−1, −2) (WNW) they are W (0, −1) and NW (−1, −1). Other knights by
  symmetry (`main.c` cases 9–16, from l.1510).
- **One slope per edge**: `Δz / L` (ratio, not percent), applied to every cell
  of the edge; sign inverted for the return direction (`-k`/towards sources).
- **Barriers**: a null speed cell, speed 0, an unknown mode, or a bicycle edge
  with a non-finite slope gives an infinite pace; the edge cost becomes null
  (`mobility.h` l.172) and the edge is skipped (`main.c` l.1788). A knight move
  across a null intermediate cell is therefore **forbidden**, as in our plan:
  not a deviation.
- **DEM NoData**: a popped cell with a null DEM is never expanded
  (`main.c` l.1038). A neighbour with a null DEM gets a null slope: walking and
  bicycling skip the edge, but **motorized ignores the slope and still assigns
  a time** to that dead-end cell. Our engine treats DEM NoData as a full
  barrier; this is a documented deviation for motorized only.
- **Limit**: `r.walk` stops once the popped cost exceeds `max_cost`
  (`main.c` l.1023), so it may keep one step past the limit;
  `amCleanTravelTime` then keeps `t <= limit`. The limit is **inclusive**.
- **Bicycle LUT**: `float` table of 201 speeds (0–100 km/h, step 0.5) × 441
  slopes (−1.2–1.0, step 0.005), interpolated bilinearly in `double` after
  clamping speed and slope (`mobility.h` l.54-108).
- Speeds come from integer scenario values (`config/config-app.R`).

## Existing test data

| Path | Content |
| --- | --- |
| `tests/accessibility/` | Motorized and bicycling demo cases, config JSON, reference data |
| `tests/referral/`, `tests/optimization/` | Later milestones |
| `docker/alpine_base/data/demo/` | Demo GRASS project (DEM, landcover, facilities) |
| `tests/README.md` | AmTests harness and patterns |

## AccessMod 5 internal plans (private, not in git)

`../accessmod_app/plans/` contains design notes that informed this project:
`P0-02_shared-mobility-kernel-bicycle-lut.md` (kernel and LUT),
`P1-01_r-accessmod-unified-engine.md` (unified GRASS engine — **superseded**
by this repository), `P2-01_road-topology-referral.md`,
`P3-01_routing-alternatives.md`.
