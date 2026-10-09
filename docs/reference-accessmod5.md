# AccessMod 5 — oracle reference

Where the behaviour this engine must reproduce (or deliberately diverge from)
lives in the official repository. Read these files to understand behaviour;
do not translate them line by line.

- Repository: <https://github.com/unige-geohealth/accessmod>
- Pinned oracle: tag **`5.9.2-beta.6`**, commit
  `e845cc96d7ba412665f02833f5d0c75ae305fb87` (branch `staging`)
- Local checkout used during design: `../accessmod_app`

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
| Bicycle LUT generator | `docker/alpine_base/modules/accessmod/generate-bicycle-lut.R` → `bicycle-lut.generated.h` + `bicycle-lut.csv` (build-time, not committed) |
| Bicycle model notes | `docker/alpine_base/modules/accessmod/bicycle-lut.md` |
| Dijkstra / neighbours | `docker/alpine_base/modules/r.walk.accessmod/main.c` (neighbour switch ~l.1240-1790) |
| Step lengths | `main.c` l.402-408 (`EW_fac`, `NS_fac`, `DIAG_fac`, `V_DIAG_fac`, `H_DIAG_fac`) |
| Kernel unit tests | `docker/alpine_base/modules/r.walk.accessmod/tests/` |

Facts established from the code (to be confirmed by oracle runs):

- **Edge cost averages pace, not speed.** Cardinal/diagonal:
  `(1/v_from + 1/v_to) · L/2`; knight: mean pace of 4 cells (from, to and the
  two cells crossed) `· L/4`, in hours × 3600. Equivalent to a harmonic mean of
  speeds.
- **One slope per edge**: `Δz / L` (ratio, not percent), applied to every cell
  of the edge; sign inverted for the return direction (`-k`/towards sources).
- **Null handling**: only the destination cell is tested for null before the
  cost call. A knight move across a null intermediate cell yields an infinite
  cost, replaced by the null value. Behaviour to characterise and decide.
- **Limit overshoot**: `r.walk` keeps the first value past the limit; the cut
  happens in `amCleanTravelTime`.
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
