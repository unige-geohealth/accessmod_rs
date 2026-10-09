//! Edge costs of the travel-time graph for `accessmod5-v1`.
//!
//! An edge joins the *origin* cell, already settled by the solver, to a
//! *target* neighbour. Its cost averages pace (1/v), not speed, over the cells
//! it touches and is returned in seconds. Mirrors `am_cost_seconds`
//! (`mobility.h` l.130-176) and the neighbour switch of
//! `r.walk.accessmod/main.c` (l.1100-1790), AccessMod 5 `5.9.2-beta.6`.
//!
//! Edges are only built through [`Edge::build`], which derives the crossed
//! cells and the length from the [`Step`] and refuses barriers, so callers
//! cannot pair a step with the wrong cells.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

/// Travel-time method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Scenario speed only; slope and transport mode ignored.
    Isotropic,
    /// Speed from the mobility model, transport mode and slope.
    Anisotropic(MobilityModel),
}

/// Set of neighbours reachable in one step; `8` or `16` in jobs.
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

    /// Number of neighbours: 8 or 16.
    pub fn count(self) -> u8 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
        }
    }
}

impl Serialize for Neighbourhood {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.count())
    }
}

impl<'de> Deserialize<'de> for Neighbourhood {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u8::deserialize(deserializer)? {
            8 => Ok(Self::Eight),
            16 => Ok(Self::Sixteen),
            other => Err(serde::de::Error::custom(format!(
                "neighbourhood must be 8 or 16, got {other}"
            ))),
        }
    }
}

impl JsonSchema for Neighbourhood {
    fn schema_name() -> Cow<'static, str> {
        "Neighbourhood".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Neighbours reachable in one step: 8 (cardinal and diagonal) or 16 (plus knight moves).",
            "type": "integer",
            "enum": [8, 16]
        })
    }
}

/// Cell size of a north-up grid, metres; both sides finite and positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellSize {
    ew_m: f64,
    ns_m: f64,
}

impl CellSize {
    /// `None` unless both sides are finite and positive.
    pub fn new(ew_m: f64, ns_m: f64) -> Option<Self> {
        (is_finite_positive(ew_m) && is_finite_positive(ns_m)).then_some(Self { ew_m, ns_m })
    }

    /// East-west resolution (column width), m.
    pub fn ew_m(self) -> f64 {
        self.ew_m
    }

    /// North-south resolution (row height), m.
    pub fn ns_m(self) -> f64 {
        self.ns_m
    }
}

fn is_finite_positive(x: f64) -> bool {
    x.is_finite() && x > 0.0
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

/// Mobility of one passable cell: its scenario transport mode and flat
/// speed, finite and positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMobility {
    mode: TransportMode,
    speed_kmh: f64,
}

impl CellMobility {
    /// `None` unless `speed_kmh` is finite and positive (a zero speed is a
    /// barrier, not a mobility).
    pub fn new(mode: TransportMode, speed_kmh: f64) -> Option<Self> {
        is_finite_positive(speed_kmh).then_some(Self { mode, speed_kmh })
    }

    /// Transport mode of the cell's land-cover class.
    pub fn mode(self) -> TransportMode {
        self.mode
    }

    /// Scenario flat speed, km/h.
    pub fn speed_kmh(self) -> f64 {
        self.speed_kmh
    }
}

/// Cells touched by an edge, all passable, in AccessMod 5 summation order.
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

/// An edge of the graph: a step and the mobility of every cell it touches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    step: Step,
    cells: EdgeCells,
}

impl Edge {
    /// Edge for `step` from a cell of mobility `origin`.
    ///
    /// `sample(offset)` returns the mobility of the cell at `offset` from the
    /// origin, or `None` for a barrier; it is called for the target and, for
    /// knight steps, for both crossed cells. Returns `Ok(None)` when any of
    /// them is a barrier: such a step has no edge.
    pub fn build<E>(
        step: Step,
        origin: CellMobility,
        mut sample: impl FnMut(Step) -> Result<Option<CellMobility>, E>,
    ) -> Result<Option<Self>, E> {
        let Some(target) = sample(step)? else {
            return Ok(None);
        };
        let cells = match step.crossed() {
            None => EdgeCells::Adjacent { origin, target },
            Some([straight, diagonal]) => {
                let (Some(straight), Some(diagonal)) = (sample(straight)?, sample(diagonal)?)
                else {
                    return Ok(None);
                };
                EdgeCells::Knight {
                    origin,
                    straight,
                    diagonal,
                    target,
                }
            }
        };
        Ok(Some(Self { step, cells }))
    }

    /// Step of this edge.
    pub fn step(&self) -> Step {
        self.step
    }

    /// Cells of this edge.
    pub fn cells(&self) -> &EdgeCells {
        &self.cells
    }

    /// Cost of the edge, seconds, or `None` if it is not finite (a speed
    /// underflowing to zero, a non-finite `dz_m`): the edge does not exist.
    ///
    /// `dz_m` is the elevation of the target minus that of the origin; it is
    /// ignored by [`Method::Isotropic`].
    pub fn cost_s(
        &self,
        method: Method,
        cell: CellSize,
        dz_m: f64,
        direction: Direction,
    ) -> Option<f64> {
        let length_m = self.step.length_m(cell);
        match method {
            Method::Isotropic => isotropic_edge_cost_s(&self.cells, length_m),
            Method::Anisotropic(model) => {
                anisotropic_edge_cost_s(model, &self.cells, length_m, dz_m, direction)
            }
        }
    }
}

/// Anisotropic edge cost, seconds. The slope `dz_m / length_m` is shared by
/// every cell of the edge and negated for [`Direction::TowardsFacilities`].
fn anisotropic_edge_cost_s(
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
/// mode and slope ignored.
fn isotropic_edge_cost_s(cells: &EdgeCells, length_m: f64) -> Option<f64> {
    edge_cost_s(cells, length_m, |c| c.speed_kmh)
}

/// `am_cost_seconds`: sum of per-cell paces weighted by `L/2` (adjacent) or
/// `L/4` (knight), in hours, then seconds. Operation order is kept for bit
/// parity with the oracle: `o + t` for adjacent, `(o + s) + (d + t)` for
/// knight steps. Speeds are positive and lengths positive, so a finite cost
/// is positive.
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
    let cost_s = cost_h * 3600.0;
    (cost_s.is_finite() && cost_s > 0.0).then_some(cost_s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MODEL: MobilityModel = MobilityModel::Accessmod5V1;

    fn cell(mode: TransportMode, speed_kmh: f64) -> CellMobility {
        CellMobility::new(mode, speed_kmh).expect("positive test speed")
    }

    fn motorized(speed_kmh: f64) -> CellMobility {
        cell(TransportMode::Motorized, speed_kmh)
    }

    fn adjacent(origin: CellMobility, target: CellMobility) -> EdgeCells {
        EdgeCells::Adjacent { origin, target }
    }

    fn size(ew_m: f64, ns_m: f64) -> CellSize {
        CellSize::new(ew_m, ns_m).expect("positive test cell size")
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
        let cell = size(30.0, 40.0);
        assert_eq!(STEPS[0].length_m(cell), 30.0);
        assert_eq!(STEPS[2].length_m(cell), 40.0);
        assert_eq!(STEPS[4].length_m(cell), 50.0);
        // NNW: 2 rows, 1 column -> sqrt(80² + 30²).
        assert_eq!(STEPS[8].length_m(cell), (6400.0f64 + 900.0).sqrt());
        // WNW: 1 row, 2 columns -> sqrt(40² + 60²).
        assert_eq!(STEPS[12].length_m(cell), (1600.0f64 + 3600.0).sqrt());
    }

    #[test]
    fn invalid_values_are_rejected() {
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert_eq!(CellMobility::new(TransportMode::Walking, bad), None);
            assert_eq!(CellSize::new(bad, 10.0), None);
            assert_eq!(CellSize::new(10.0, bad), None);
        }
    }

    #[test]
    fn neighbourhood_serialises_as_a_number() {
        let json = serde_json::to_string(&Neighbourhood::Sixteen).expect("serialise");
        assert_eq!(json, "16");
        let back: Neighbourhood = serde_json::from_str("8").expect("deserialise");
        assert_eq!(back, Neighbourhood::Eight);
        assert!(serde_json::from_str::<Neighbourhood>("4").is_err());
        assert!(serde_json::from_str::<Neighbourhood>("\"8\"").is_err());
    }

    #[test]
    fn build_refuses_barriers_and_orders_knight_cells() {
        let origin = motorized(1.0);
        let speed_of = |s: Step| f64::from(s.d_row * 10 + s.d_col) + 50.0;
        let open = |s: Step| Ok::<_, ()>(CellMobility::new(TransportMode::Motorized, speed_of(s)));
        let nnw = STEPS[8];
        let edge = Edge::build(nnw, origin, open)
            .expect("no error")
            .expect("edge");
        let EdgeCells::Knight {
            straight,
            diagonal,
            target,
            ..
        } = *edge.cells()
        else {
            panic!("knight step builds a knight edge");
        };
        assert_eq!(straight.speed_kmh(), speed_of(Step::new(-1, 0)));
        assert_eq!(diagonal.speed_kmh(), speed_of(Step::new(-1, -1)));
        assert_eq!(target.speed_kmh(), speed_of(nnw));

        let blocked = |s: Step| Ok::<_, ()>((s != Step::new(-1, -1)).then(|| motorized(5.0)));
        assert_eq!(Edge::build(nnw, origin, blocked), Ok(None));
        assert!(
            Edge::build(STEPS[4], origin, blocked)
                .expect("ok")
                .is_none()
        );
        assert!(
            Edge::build(STEPS[0], origin, blocked)
                .expect("ok")
                .is_some()
        );
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
        // 50 m in each cell: 36 + 18 + 9 + 4.5 s.
        let cost = isotropic_edge_cost_s(&cells, 200.0).unwrap_or(f64::NAN);
        assert!((cost - 67.5).abs() < 1e-12, "{cost}");
    }

    #[test]
    fn edge_cost_uses_the_step_length() {
        let edge = Edge::build(STEPS[4], motorized(5.0), |_| {
            Ok::<_, ()>(Some(motorized(5.0)))
        })
        .expect("ok")
        .expect("edge");
        let cost = edge.cost_s(
            Method::Isotropic,
            size(30.0, 40.0),
            0.0,
            Direction::FromFacilities,
        );
        // 50 m at 5 km/h = 36 s.
        assert!((cost.unwrap_or(f64::NAN) - 36.0).abs() < 1e-12);
    }

    #[test]
    fn non_finite_costs_mean_no_edge() {
        let walk = cell(TransportMode::Walking, 5.0);
        // Tobler's speed underflows to zero on an absurd slope.
        let cells = adjacent(walk, walk);
        let cost = anisotropic_edge_cost_s(MODEL, &cells, 1.0, 1e6, Direction::FromFacilities);
        assert_eq!(cost, None);
        let bike = cell(TransportMode::Bicycling, 18.0);
        let cost = anisotropic_edge_cost_s(
            MODEL,
            &adjacent(bike, bike),
            100.0,
            f64::NAN,
            Direction::FromFacilities,
        );
        assert_eq!(cost, None);
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
        (any_mode(), 0.5f64..120.0).prop_map(|(mode, speed_kmh)| cell(mode, speed_kmh))
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
