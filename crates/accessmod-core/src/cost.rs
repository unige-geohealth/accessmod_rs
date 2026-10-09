//! Edge costs of the travel-time graph for `accessmod5-v1`.
//!
//! An edge joins the *origin* cell, already settled by the solver, to a
//! *target* neighbour. Its cost averages pace (1/v), not speed, over the cells
//! it touches and is returned in seconds. Mirrors `am_cost_seconds`
//! (`mobility.h` l.130-176) and the neighbour switch of
//! `r.walk.accessmod/main.c` (l.1100-1790), AccessMod 5 `5.9.2-beta.6`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::mobility::{MobilityModel, TransportMode};

/// Direction of travel relative to the facilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Travel from the facilities to every cell.
    FromFacilities,
    /// Travel from every cell to the facilities. The solver still expands
    /// from the facilities; only the slope sign of each edge is inverted.
    TowardsFacilities,
}

/// Set of neighbours reachable in one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Neighbourhood {
    /// Cardinal and diagonal neighbours.
    Eight,
    /// The eight above plus the eight knight moves.
    Sixteen,
}

impl Neighbourhood {
    /// Steps of this neighbourhood, in AccessMod 5 order.
    pub fn steps(self) -> &'static [Step] {
        match self {
            Self::Eight => &STEPS[..8],
            Self::Sixteen => &STEPS,
        }
    }
}

/// Cell size of a north-up grid, metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellSize {
    /// East-west resolution (column width), m.
    pub ew_m: f64,
    /// North-south resolution (row height), m.
    pub ns_m: f64,
}

/// Geometric kind of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKind {
    /// One row or one column.
    Cardinal,
    /// One row and one column.
    Diagonal,
    /// Two rows and one column, or one row and two columns.
    Knight,
}

/// Offset from the origin cell to a neighbour; rows grow southwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Step {
    /// Row offset (negative = north).
    pub d_row: i8,
    /// Column offset (negative = west).
    pub d_col: i8,
}

/// All steps, in the order of the AccessMod 5 neighbour switch (cases 1–16):
/// W, E, N, S, NW, NE, SE, SW, NNW, NNE, SSE, SSW, WNW, ENE, ESE, WSW.
/// [`Step::code`] is the 1-based position in this array.
pub const STEPS: [Step; 16] = [
    Step::new(0, -1),
    Step::new(0, 1),
    Step::new(-1, 0),
    Step::new(1, 0),
    Step::new(-1, -1),
    Step::new(-1, 1),
    Step::new(1, 1),
    Step::new(1, -1),
    Step::new(-2, -1),
    Step::new(-2, 1),
    Step::new(2, 1),
    Step::new(2, -1),
    Step::new(-1, -2),
    Step::new(-1, 2),
    Step::new(1, 2),
    Step::new(1, -2),
];

impl Step {
    const fn new(d_row: i8, d_col: i8) -> Self {
        Self { d_row, d_col }
    }

    /// Geometric kind.
    pub fn kind(self) -> StepKind {
        match (self.d_row.abs(), self.d_col.abs()) {
            (1, 1) => StepKind::Diagonal,
            (2, 1) | (1, 2) => StepKind::Knight,
            _ => StepKind::Cardinal,
        }
    }

    /// Code of this step, 1–16, in AccessMod 5 neighbour order; 0 if the
    /// step is not one of [`STEPS`].
    pub fn code(self) -> u8 {
        STEPS
            .iter()
            .position(|s| *s == self)
            .map_or(0, |i| i as u8 + 1)
    }

    /// For a knight step, the two cells it crosses, in AccessMod 5 summation
    /// order: first one step along the long axis (straight), then the
    /// diagonal neighbour on the same side as the target. `None` otherwise.
    ///
    /// Example: NNW (−2, −1) crosses N (−1, 0) then NW (−1, −1).
    pub fn crossed(self) -> Option<[Step; 2]> {
        if self.kind() != StepKind::Knight {
            return None;
        }
        let (dr, dc) = (self.d_row, self.d_col);
        Some(if dr.abs() == 2 {
            [Step::new(dr.signum(), 0), Step::new(dr.signum(), dc)]
        } else {
            [Step::new(0, dc.signum()), Step::new(dr, dc.signum())]
        })
    }

    /// Planar length of the step, m. Same expressions as `main.c`
    /// l.402-408 (`EW_fac`, `NS_fac`, `DIAG_fac`, `V_DIAG_fac`, `H_DIAG_fac`).
    pub fn length_m(self, cell: CellSize) -> f64 {
        let (ew, ns) = (cell.ew_m, cell.ns_m);
        match (self.d_row.abs(), self.d_col.abs()) {
            (0, _) => ew,
            (_, 0) => ns,
            (1, 1) => (ns * ns + ew * ew).sqrt(),
            (2, _) => (4.0 * ns * ns + ew * ew).sqrt(),
            _ => (ns * ns + 4.0 * ew * ew).sqrt(),
        }
    }
}

/// Mobility of one passable cell: its scenario transport mode and flat speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMobility {
    /// Transport mode of the cell's land-cover class.
    pub mode: TransportMode,
    /// Scenario flat speed, km/h, finite and > 0 for a passable cell.
    pub speed_kmh: f64,
}

/// Cells touched by an edge, all passable. A knight step touching a barrier
/// has no edge: the caller must not build one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeCells {
    /// Cardinal or diagonal step.
    Adjacent {
        /// Settled cell.
        origin: CellMobility,
        /// Neighbour being relaxed.
        target: CellMobility,
    },
    /// Knight step.
    Knight {
        /// Settled cell.
        origin: CellMobility,
        /// Crossed cell one step along the long axis.
        straight: CellMobility,
        /// Crossed diagonal cell.
        diagonal: CellMobility,
        /// Neighbour being relaxed.
        target: CellMobility,
    },
}

/// Anisotropic edge cost, seconds.
///
/// - `length_m`: planar step length ([`Step::length_m`]);
/// - `dz_m`: elevation of the target minus elevation of the origin, finite
///   (a NoData DEM cell is a barrier and never reaches this function).
///
/// The slope `dz_m / length_m` is shared by every cell of the edge and
/// negated for [`Direction::TowardsFacilities`]. Returns `None` when the cost
/// is not finite (a cell with zero speed): the edge does not exist.
pub fn anisotropic_edge_cost_s(
    model: MobilityModel,
    cells: &EdgeCells,
    length_m: f64,
    dz_m: f64,
    direction: Direction,
) -> Option<f64> {
    let slope = match direction {
        Direction::FromFacilities => dz_m / length_m,
        Direction::TowardsFacilities => -(dz_m / length_m),
    };
    edge_cost_s(cells, length_m, |c| {
        model.speed_kmh(c.mode, c.speed_kmh, slope)
    })
}

/// Isotropic edge cost, seconds: the scenario speed of each cell, transport
/// mode and slope ignored. Same pace averaging as
/// [`anisotropic_edge_cost_s`].
pub fn isotropic_edge_cost_s(cells: &EdgeCells, length_m: f64) -> Option<f64> {
    edge_cost_s(cells, length_m, |c| c.speed_kmh)
}

/// `am_cost_seconds`: sum of per-cell paces weighted by `L/2` (adjacent) or
/// `L/4` (knight), in hours, then seconds. Operation order is kept for bit
/// parity with the oracle: `((o + t)` for adjacent, `(o + s) + (d + t)` for
/// knight steps.
fn edge_cost_s(
    cells: &EdgeCells,
    length_m: f64,
    speed_kmh: impl Fn(&CellMobility) -> f64,
) -> Option<f64> {
    let pace_h = |cell: &CellMobility, divisor: f64| 1.0 / (speed_kmh(cell) / divisor);
    let cost_h = match cells {
        EdgeCells::Adjacent { origin, target } => {
            let divisor = length_m / 2000.0;
            pace_h(origin, divisor) + pace_h(target, divisor)
        }
        EdgeCells::Knight {
            origin,
            straight,
            diagonal,
            target,
        } => {
            let divisor = length_m / 4000.0;
            let mut cost = pace_h(origin, divisor) + pace_h(straight, divisor);
            cost += pace_h(diagonal, divisor) + pace_h(target, divisor);
            cost
        }
    };
    cost_h.is_finite().then_some(cost_h * 3600.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MODEL: MobilityModel = MobilityModel::Accessmod5V1;

    fn cell(mode: TransportMode, speed_kmh: f64) -> CellMobility {
        CellMobility { mode, speed_kmh }
    }

    fn motorized(speed_kmh: f64) -> CellMobility {
        cell(TransportMode::Motorized, speed_kmh)
    }

    fn adjacent(origin: CellMobility, target: CellMobility) -> EdgeCells {
        EdgeCells::Adjacent { origin, target }
    }

    #[test]
    fn step_codes_follow_accessmod5_order() {
        for (i, step) in STEPS.iter().enumerate() {
            assert_eq!(usize::from(step.code()), i + 1);
        }
        assert_eq!(Step::new(0, 0).code(), 0);
        assert_eq!(Neighbourhood::Eight.steps().len(), 8);
        assert_eq!(Neighbourhood::Sixteen.steps().len(), 16);
        assert!(
            Neighbourhood::Eight
                .steps()
                .iter()
                .all(|s| s.kind() != StepKind::Knight)
        );
    }

    #[test]
    fn knight_crossed_cells_match_main_c() {
        // (step, straight, diagonal) from main.c cases 9-16.
        let expected = [
            ((-2, -1), (-1, 0), (-1, -1)),
            ((-2, 1), (-1, 0), (-1, 1)),
            ((2, 1), (1, 0), (1, 1)),
            ((2, -1), (1, 0), (1, -1)),
            ((-1, -2), (0, -1), (-1, -1)),
            ((-1, 2), (0, 1), (-1, 1)),
            ((1, 2), (0, 1), (1, 1)),
            ((1, -2), (0, -1), (1, -1)),
        ];
        for (step, straight, diagonal) in expected {
            let step = Step::new(step.0, step.1);
            let crossed = step
                .crossed()
                .map(|[s, d]| ((s.d_row, s.d_col), (d.d_row, d.d_col)));
            assert_eq!(crossed, Some((straight, diagonal)), "{step:?}");
        }
        assert_eq!(STEPS[0].crossed(), None);
    }

    #[test]
    fn step_lengths() {
        let cell = CellSize {
            ew_m: 30.0,
            ns_m: 40.0,
        };
        assert_eq!(STEPS[0].length_m(cell), 30.0);
        assert_eq!(STEPS[2].length_m(cell), 40.0);
        assert_eq!(STEPS[4].length_m(cell), 50.0);
        // NNW: 2 rows, 1 column -> sqrt(80² + 30²).
        assert_eq!(STEPS[8].length_m(cell), (6400.0f64 + 900.0).sqrt());
        // WNW: 1 row, 2 columns -> sqrt(40² + 60²).
        assert_eq!(STEPS[12].length_m(cell), (1600.0f64 + 3600.0).sqrt());
    }

    #[test]
    fn adjacent_cost_by_hand() {
        // 100 m at 5 km/h = 0.02 h = 72 s.
        let cost = isotropic_edge_cost_s(&adjacent(motorized(5.0), motorized(5.0)), 100.0);
        assert_eq!(cost, Some(72.0));
        // Pace average: 50 m at 5 km/h (36 s) + 50 m at 10 km/h (18 s).
        let cost = isotropic_edge_cost_s(&adjacent(motorized(5.0), motorized(10.0)), 100.0);
        assert!((cost.unwrap_or(f64::NAN) - 54.0).abs() < 1e-12);
    }

    #[test]
    fn knight_cost_by_hand() {
        let cells = EdgeCells::Knight {
            origin: motorized(5.0),
            straight: motorized(10.0),
            diagonal: motorized(20.0),
            target: motorized(40.0),
        };
        let length = 200.0;
        // 50 m in each cell: 36 + 18 + 9 + 4.5 s.
        let cost = isotropic_edge_cost_s(&cells, length).unwrap_or(f64::NAN);
        assert!((cost - 67.5).abs() < 1e-12, "{cost}");
    }

    #[test]
    fn zero_speed_is_a_barrier() {
        let cells = adjacent(motorized(5.0), motorized(0.0));
        assert_eq!(isotropic_edge_cost_s(&cells, 100.0), None);
        let cells = adjacent(motorized(5.0), cell(TransportMode::Bicycling, 0.0));
        assert_eq!(
            anisotropic_edge_cost_s(MODEL, &cells, 100.0, 0.0, Direction::FromFacilities),
            None
        );
    }

    #[test]
    fn towards_facilities_inverts_slope() {
        let walk = cell(TransportMode::Walking, 5.0);
        let cells = adjacent(walk, walk);
        let up_from =
            anisotropic_edge_cost_s(MODEL, &cells, 100.0, 10.0, Direction::FromFacilities);
        let down_to =
            anisotropic_edge_cost_s(MODEL, &cells, 100.0, -10.0, Direction::TowardsFacilities);
        let down_from =
            anisotropic_edge_cost_s(MODEL, &cells, 100.0, -10.0, Direction::FromFacilities);
        assert_eq!(up_from, down_to);
        assert_ne!(up_from, down_from);
    }

    #[test]
    fn walking_uses_one_slope_for_the_whole_edge() {
        let walk = cell(TransportMode::Walking, 5.0);
        let cost = anisotropic_edge_cost_s(
            MODEL,
            &adjacent(walk, walk),
            100.0,
            10.0,
            Direction::FromFacilities,
        );
        let v = crate::mobility::walking_speed_kmh(5.0, 0.1);
        let expected = 100.0 / 1000.0 / v * 3600.0;
        assert!((cost.unwrap_or(f64::NAN) - expected).abs() < 1e-9);
    }

    #[test]
    fn mixed_modes_use_each_cell_mode() {
        let cells = adjacent(motorized(60.0), cell(TransportMode::Walking, 5.0));
        let cost = anisotropic_edge_cost_s(MODEL, &cells, 100.0, 0.0, Direction::FromFacilities);
        // 50 m at 60 km/h (3 s) + 50 m at 5 km/h (36 s).
        assert!((cost.unwrap_or(f64::NAN) - 39.0).abs() < 1e-9);
    }

    fn any_mode() -> impl Strategy<Value = TransportMode> {
        prop_oneof![
            Just(TransportMode::Walking),
            Just(TransportMode::Bicycling),
            Just(TransportMode::Motorized),
        ]
    }

    fn any_cell() -> impl Strategy<Value = CellMobility> {
        (any_mode(), 0.5f64..120.0).prop_map(|(mode, speed_kmh)| CellMobility { mode, speed_kmh })
    }

    proptest! {
        #[test]
        fn costs_are_positive_and_finite(
            o in any_cell(), t in any_cell(), length in 1.0f64..1000.0, slope in -0.5f64..0.5,
        ) {
            let cells = adjacent(o, t);
            for direction in [Direction::FromFacilities, Direction::TowardsFacilities] {
                let cost = anisotropic_edge_cost_s(MODEL, &cells, length, slope * length, direction);
                prop_assert!(cost.is_some_and(|c| c > 0.0 && c.is_finite()));
            }
            let iso = isotropic_edge_cost_s(&cells, length);
            prop_assert!(iso.is_some_and(|c| c > 0.0 && c.is_finite()));
        }

        #[test]
        fn isotropic_matches_flat_motorized(
            a in 0.5f64..120.0, b in 0.5f64..120.0, length in 1.0f64..1000.0,
        ) {
            let cells = adjacent(motorized(a), motorized(b));
            let aniso = anisotropic_edge_cost_s(MODEL, &cells, length, 37.0, Direction::FromFacilities);
            prop_assert_eq!(aniso, isotropic_edge_cost_s(&cells, length));
        }

        #[test]
        fn isotropic_is_symmetric(
            a in 0.5f64..120.0, b in 0.5f64..120.0, length in 1.0f64..1000.0,
        ) {
            let ab = isotropic_edge_cost_s(&adjacent(motorized(a), motorized(b)), length);
            let ba = isotropic_edge_cost_s(&adjacent(motorized(b), motorized(a)), length);
            prop_assert!((ab.unwrap_or(f64::NAN) - ba.unwrap_or(f64::NAN)).abs() <= 1e-12 * ab.unwrap_or(1.0));
        }
    }
}
