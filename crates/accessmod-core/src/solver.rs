//! Deterministic multi-source Dijkstra over tiled grids.
//!
//! One pass produces, for every reachable cell, the travel time (s), the
//! planar length of the least-time path (m), the nearest facility (cost
//! allocation) and the step that reached it.
//!
//! Ties are broken by (travel time, path distance, facility order, linear
//! cell index), in this order, so results never depend on storage, cache
//! capacity or evaluation order. The limit is inclusive and exact: no cell
//! beyond it is ever assigned (`r.walk` overshoots by one step and AccessMod
//! 5 cuts afterwards with the same inclusive rule).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub use crate::cost::Method;
use crate::cost::{CellMobility, CellSize, Direction, Edge, Neighbourhood, Step};
use crate::store::{CellCodec, StoreError, TiledGrid};

/// Solver input for one cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputCell {
    /// Index into the scenario table, or [`InputCell::BARRIER`].
    pub slot: u16,
    /// Elevation, m; NaN for DEM NoData (a barrier).
    pub dem_m: f64,
}

impl InputCell {
    /// Slot of a cell without scenario: NoData land cover or class without
    /// scenario entry.
    pub const BARRIER: u16 = u16::MAX;
    /// Input of a cell outside every scenario.
    pub const NODATA: Self = Self {
        slot: Self::BARRIER,
        dem_m: f64::NAN,
    };

    /// Whether the cell can be entered.
    pub fn is_passable(&self) -> bool {
        self.slot != Self::BARRIER && self.dem_m.is_finite()
    }
}

impl CellCodec for InputCell {
    const BYTES: usize = 10;
    fn encode(self, out: &mut [u8]) {
        out[..2].copy_from_slice(&self.slot.to_le_bytes());
        out[2..].copy_from_slice(&self.dem_m.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            slot: u16::decode(&bytes[..2]),
            dem_m: f64::decode(&bytes[2..]),
        }
    }
}

/// Solver state of one cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolverCell {
    /// Travel time, s; `+∞` if unreached.
    pub time_s: f64,
    /// Planar length of the least-time path, m; `+∞` if unreached.
    pub distance_m: f64,
    /// 1-based order of the nearest facility in the facility list; 0 if
    /// unreached.
    pub nearest: u32,
    /// Code of the step that reached the cell ([`Step::code`]); 0 for a
    /// facility cell or an unreached cell.
    pub step: u8,
    /// Whether the cell's time is final.
    pub settled: bool,
}

impl SolverCell {
    /// State of a cell not reached (yet).
    pub const UNREACHED: Self = Self {
        time_s: f64::INFINITY,
        distance_m: f64::INFINITY,
        nearest: 0,
        step: 0,
        settled: false,
    };

    /// Whether a facility reached the cell.
    pub fn is_reached(&self) -> bool {
        self.nearest != 0
    }

    fn key(&self) -> (f64, f64, u32) {
        (self.time_s, self.distance_m, self.nearest)
    }
}

impl CellCodec for SolverCell {
    const BYTES: usize = 22;
    fn encode(self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.time_s.to_le_bytes());
        out[8..16].copy_from_slice(&self.distance_m.to_le_bytes());
        out[16..20].copy_from_slice(&self.nearest.to_le_bytes());
        out[20] = self.step;
        out[21] = u8::from(self.settled);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            time_s: f64::decode(&bytes[..8]),
            distance_m: f64::decode(&bytes[8..16]),
            nearest: u32::decode(&bytes[16..20]),
            step: bytes[20],
            settled: bytes[21] != 0,
        }
    }
}

/// Parameters of one solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolverConfig {
    /// Travel-time method.
    pub method: Method,
    /// Direction of travel.
    pub direction: Direction,
    /// Neighbours reachable in one step.
    pub neighbourhood: Neighbourhood,
    /// Cell size of the grid.
    pub cell: CellSize,
    /// Inclusive travel-time limit, s.
    pub limit_s: Option<f64>,
}

/// Facility cell, in facility-layer order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facility {
    /// Grid row.
    pub row: usize,
    /// Grid column.
    pub col: usize,
}

/// Solver errors.
#[derive(Debug, thiserror::Error)]
pub enum SolveError {
    /// Tiled storage failure.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A facility lies outside the grid or on a barrier. Validation should
    /// have rejected or skipped it.
    #[error("facility #{ordinal} at cell ({row}, {col}) is {reason}")]
    Facility {
        /// 1-based order in the facility list.
        ordinal: u32,
        /// Grid row.
        row: usize,
        /// Grid column.
        col: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// Input and state grids do not have the same geometry, or the scenario
    /// table is too large.
    #[error("invalid solver input: {0}")]
    Invalid(&'static str),
    /// Cancelled by the caller.
    #[error("cancelled")]
    Cancelled,
}

/// Counters of one solve, for `run.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SolveStats {
    /// Cells settled.
    pub settled: u64,
    /// Heap entries popped (settled + stale).
    pub pops: u64,
    /// Largest number of heap entries at once.
    pub heap_peak: usize,
    /// Facilities sharing a cell with an earlier facility:
    /// (ordinal, ordinal kept). They allocate no cell.
    pub shared_cells: Vec<(u32, u32)>,
}

/// Min-heap entry ordered by the documented tie-break.
#[derive(Debug, Clone, Copy)]
struct Entry {
    time_s: f64,
    distance_m: f64,
    nearest: u32,
    index: u64,
}

impl Entry {
    fn cmp_key(&self, other: &Self) -> Ordering {
        self.time_s
            .total_cmp(&other.time_s)
            .then(self.distance_m.total_cmp(&other.distance_m))
            .then(self.nearest.cmp(&other.nearest))
            .then(self.index.cmp(&other.index))
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_key(other) == Ordering::Equal
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    // Reversed: `BinaryHeap` is a max-heap.
    fn cmp(&self, other: &Self) -> Ordering {
        other.cmp_key(self)
    }
}

/// Lexicographic `(time, distance, facility order)` comparison with a total
/// order on floats.
fn better(candidate: (f64, f64, u32), current: (f64, f64, u32)) -> bool {
    candidate
        .0
        .total_cmp(&current.0)
        .then(candidate.1.total_cmp(&current.1))
        .then(candidate.2.cmp(&current.2))
        == Ordering::Less
}

/// How often the cancellation callback is polled, in heap pops.
const CANCEL_POLL: u64 = 4096;

/// Run the solver.
///
/// - `inputs`: one [`InputCell`] per cell;
/// - `scenario`: mobility of each slot used in `inputs`;
/// - `facilities`: in facility-layer order; the k-th has ordinal k + 1;
/// - `state`: grid of the same geometry, every cell
///   [`SolverCell::UNREACHED`]; holds the result;
/// - `cancelled`: polled regularly; returning `true` aborts with
///   [`SolveError::Cancelled`].
pub fn solve(
    inputs: &mut TiledGrid<InputCell>,
    scenario: &[CellMobility],
    facilities: &[Facility],
    config: &SolverConfig,
    state: &mut TiledGrid<SolverCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<SolveStats, SolveError> {
    let geometry = inputs.geometry();
    if state.geometry() != geometry {
        return Err(SolveError::Invalid(
            "input and state grids differ in geometry",
        ));
    }
    if scenario.len() >= usize::from(InputCell::BARRIER) {
        return Err(SolveError::Invalid("too many scenario entries"));
    }
    let (rows, cols) = (geometry.rows(), geometry.cols());
    let mut stats = SolveStats::default();
    let mut heap = BinaryHeap::new();

    for (k, facility) in facilities.iter().enumerate() {
        let ordinal =
            u32::try_from(k + 1).map_err(|_| SolveError::Invalid("too many facilities"))?;
        let (row, col) = (facility.row, facility.col);
        let error = |reason| SolveError::Facility {
            ordinal,
            row,
            col,
            reason,
        };
        if row >= rows || col >= cols {
            return Err(error("outside the grid"));
        }
        let input = inputs.get(row, col)?;
        if !input.is_passable() || scenario.get(usize::from(input.slot)).is_none() {
            return Err(error("on a barrier"));
        }
        let current = state.get(row, col)?;
        if current.is_reached() {
            stats.shared_cells.push((ordinal, current.nearest));
            continue;
        }
        state.set(
            row,
            col,
            SolverCell {
                time_s: 0.0,
                distance_m: 0.0,
                nearest: ordinal,
                step: 0,
                settled: false,
            },
        )?;
        heap.push(Entry {
            time_s: 0.0,
            distance_m: 0.0,
            nearest: ordinal,
            index: (row * cols + col) as u64,
        });
    }
    stats.heap_peak = heap.len();

    let steps = config.neighbourhood.steps();
    while let Some(entry) = heap.pop() {
        stats.pops += 1;
        if stats.pops.is_multiple_of(CANCEL_POLL) && cancelled() {
            return Err(SolveError::Cancelled);
        }
        let (row, col) = (
            (entry.index / cols as u64) as usize,
            (entry.index % cols as u64) as usize,
        );
        let mut current = state.get(row, col)?;
        if current.settled || current.key() != (entry.time_s, entry.distance_m, entry.nearest) {
            continue;
        }
        current.settled = true;
        state.set(row, col, current)?;
        stats.settled += 1;

        let origin = inputs.get(row, col)?;
        let origin_mobility = mobility(scenario, origin).expect(
            "settled cells are passable: facilities are checked, edges need passable targets",
        );
        for &step in steps {
            let Some((t_row, t_col)) = offset(row, col, step, rows, cols) else {
                continue;
            };
            let target = inputs.get(t_row, t_col)?;
            if !target.is_passable() {
                continue;
            }
            let target_state = state.get(t_row, t_col)?;
            if target_state.settled {
                continue;
            }
            // Offsets of the target and crossed cells lie between origin and
            // target, hence inside the grid once the target is.
            let edge = Edge::build(step, origin_mobility, |offset_step| {
                let (r, c) = offset(row, col, offset_step, rows, cols)
                    .expect("edge cells lie inside the grid when the target does");
                let input = inputs.get(r, c)?;
                Ok::<_, StoreError>(mobility(scenario, input))
            })?;
            let Some(edge) = edge else {
                continue;
            };
            let dz_m = target.dem_m - origin.dem_m;
            let length_m = step.length_m(config.cell);
            let Some(cost_s) = edge.cost_s(config.method, config.cell, dz_m, config.direction)
            else {
                continue;
            };
            let time_s = current.time_s + cost_s;
            let distance_m = current.distance_m + length_m;
            // An overflowing sum is unreachable, never a reached cell at +inf.
            if !time_s.is_finite() || !distance_m.is_finite() {
                continue;
            }
            if config.limit_s.is_some_and(|limit| time_s > limit) {
                continue;
            }
            let candidate = SolverCell {
                time_s,
                distance_m,
                nearest: current.nearest,
                step: step.code(),
                settled: false,
            };
            if better(candidate.key(), target_state.key()) {
                state.set(t_row, t_col, candidate)?;
                heap.push(Entry {
                    time_s,
                    distance_m: candidate.distance_m,
                    nearest: candidate.nearest,
                    index: (t_row * cols + t_col) as u64,
                });
                stats.heap_peak = stats.heap_peak.max(heap.len());
            }
        }
    }
    Ok(stats)
}

/// Cell reached by `step` from `(row, col)`, if inside the grid.
fn offset(row: usize, col: usize, step: Step, rows: usize, cols: usize) -> Option<(usize, usize)> {
    let r = row.checked_add_signed(isize::from(step.d_row))?;
    let c = col.checked_add_signed(isize::from(step.d_col))?;
    (r < rows && c < cols).then_some((r, c))
}

/// Mobility of a cell, `None` for a barrier.
fn mobility(scenario: &[CellMobility], input: InputCell) -> Option<CellMobility> {
    if input.is_passable() {
        scenario.get(usize::from(input.slot)).copied()
    } else {
        None
    }
}
