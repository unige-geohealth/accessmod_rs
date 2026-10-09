//! Many-to-many travel time and path distance between facilities.
//!
//! One exact search per source, stopped as soon as it has settled what it
//! needs, sources spread over worker threads (`docs/referral.md`):
//!
//! - travel always goes from an origin to a destination;
//! - searches start from the smaller side: `time(o → d)` equals the search
//!   from `d` with inverted slopes ([`Direction::TowardsFacilities`]), so
//!   `N × M` pairs cost `min(N, M)` searches (the last bits of knight-step
//!   sums may differ between the two sides: the summation order of the four
//!   cells changes);
//! - with `nearest_only`, searches start from the origins and stop at the
//!   first destination settled;
//! - results never depend on the number of workers or their scheduling.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::cost::{CellMobility, CellSize, Direction, Method, Neighbourhood};
use crate::solver::{
    Facility, Flow, InputCell, InputReader, SolveError, SolverCell, SolverConfig, solve_until,
};
use crate::store::{Frozen, FrozenReader, StoreError, TileGeometry, TiledGrid};

/// Parameters of a referral analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferralConfig {
    /// Travel-time method.
    pub method: Method,
    /// Neighbours reachable in one step.
    pub neighbourhood: Neighbourhood,
    /// Cell size.
    pub cell: CellSize,
    /// Inclusive travel-time limit, s.
    pub limit_s: Option<f64>,
    /// Only the nearest destination (by time) of each origin.
    pub nearest_only: bool,
}

/// Time and path length of one origin → destination pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pair {
    /// Travel time, s.
    pub time_s: f64,
    /// Planar length of the least-time path, m.
    pub distance_m: f64,
}

/// Result of a referral analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferralResult {
    /// `pairs[o][d]`: origin `o` to destination `d`, `None` if not reached
    /// (barrier, limit, or not the nearest with `nearest_only`).
    pub pairs: Vec<Vec<Option<Pair>>>,
    /// Whether searches started from the destinations.
    pub reversed: bool,
    /// Number of searches.
    pub searches: usize,
    /// Cells settled over all searches.
    pub settled: u64,
}

/// Memory given to each worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerBudget {
    /// Resident state tiles per worker.
    pub state_tiles: usize,
    /// Resident input tiles per worker (file-backed inputs only).
    pub input_tiles: usize,
}

impl InputReader for FrozenReader<'_, InputCell> {
    fn geometry(&self) -> TileGeometry {
        FrozenReader::geometry(self)
    }

    #[inline]
    fn input(&mut self, row: usize, col: usize) -> Result<InputCell, StoreError> {
        self.get(row, col)
    }
}

/// Outcome of one search: its source position and its row of pairs.
type SearchRow = (usize, Vec<Option<Pair>>, u64);

/// Run the referral analysis.
///
/// `origins` and `destinations` must be passable cells (checked by the
/// solver). `workers` threads run searches concurrently; `cancelled` is
/// polled by every search.
#[allow(clippy::too_many_arguments)] // reason: explicit inputs of one analysis
pub fn referral(
    inputs: &Frozen<InputCell>,
    scenario: &[CellMobility],
    origins: &[Facility],
    destinations: &[Facility],
    config: &ReferralConfig,
    workers: usize,
    budget: WorkerBudget,
    scratch_dir: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<ReferralResult, SolveError> {
    let reversed = !config.nearest_only && destinations.len() < origins.len();
    let (sources, targets, direction) = if reversed {
        (destinations, origins, Direction::TowardsFacilities)
    } else {
        (origins, destinations, Direction::FromFacilities)
    };
    let geometry = inputs.geometry();
    let cols = geometry.cols();
    // Target cells -> target positions (several facilities may share a cell).
    let mut target_cells: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (k, t) in targets.iter().enumerate() {
        target_cells
            .entry(t.row * cols + t.col)
            .or_default()
            .push(k);
    }
    let solver_config = SolverConfig {
        method: config.method,
        direction,
        neighbourhood: config.neighbourhood,
        cell: config.cell,
        limit_s: config.limit_s,
    };

    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let rows: Mutex<Vec<SearchRow>> = Mutex::new(Vec::with_capacity(sources.len()));
    let errors: Mutex<Vec<(usize, SolveError)>> = Mutex::new(Vec::new());
    let stop = || cancelled() || failed.load(Ordering::Relaxed);

    let worker = || -> Result<(), (usize, SolveError)> {
        let mut reader = inputs
            .reader(budget.input_tiles.max(1))
            .map_err(|e| (0, SolveError::Store(e)))?;
        loop {
            let s = next.fetch_add(1, Ordering::Relaxed);
            if s >= sources.len() || stop() {
                return Ok(());
            }
            let row = search(
                &mut reader,
                scenario,
                sources[s],
                targets.len(),
                &target_cells,
                &solver_config,
                config.nearest_only,
                geometry,
                budget.state_tiles,
                scratch_dir,
                &stop,
            )
            .map_err(|e| (s, e))?;
            rows.lock()
                .expect("no worker panics while holding the lock")
                .push((s, row.0, row.1));
        }
    };

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers.clamp(1, sources.len().max(1)))
            .map(|_| {
                scope.spawn(|| {
                    if let Err(e) = worker() {
                        failed.store(true, Ordering::Relaxed);
                        errors
                            .lock()
                            .expect("no worker panics while holding the lock")
                            .push(e);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("referral workers do not panic");
        }
    });

    // Report the error of the first failing source, whatever the schedule;
    // a cancellation only wins when nothing else failed.
    let mut errors = errors.into_inner().expect("workers finished");
    errors.sort_by_key(|(s, e)| (matches!(e, SolveError::Cancelled), *s));
    if let Some((_, error)) = errors.into_iter().next() {
        return Err(error);
    }
    if cancelled() {
        return Err(SolveError::Cancelled);
    }

    let mut rows = rows.into_inner().expect("workers finished");
    rows.sort_by_key(|(s, _, _)| *s);
    let settled = rows.iter().map(|(_, _, n)| n).sum();
    let searches = rows.len();
    let by_source: Vec<Vec<Option<Pair>>> = rows.into_iter().map(|(_, row, _)| row).collect();
    let pairs = if reversed {
        (0..origins.len())
            .map(|o| (0..destinations.len()).map(|d| by_source[d][o]).collect())
            .collect()
    } else {
        by_source
    };
    Ok(ReferralResult {
        pairs,
        reversed,
        searches,
        settled,
    })
}

/// One search from `source`: time and distance to every target, or to the
/// first one settled when `first_only`.
#[allow(clippy::too_many_arguments)] // reason: private helper of `referral`
fn search(
    reader: &mut FrozenReader<'_, InputCell>,
    scenario: &[CellMobility],
    source: Facility,
    n_targets: usize,
    target_cells: &BTreeMap<usize, Vec<usize>>,
    config: &SolverConfig,
    first_only: bool,
    geometry: TileGeometry,
    state_tiles: usize,
    scratch_dir: &Path,
    stop: &dyn Fn() -> bool,
) -> Result<(Vec<Option<Pair>>, u64), SolveError> {
    let cols = geometry.cols();
    let mut state = TiledGrid::new(
        geometry,
        SolverCell::UNREACHED,
        state_tiles.max(1),
        scratch_dir,
        "referral",
    )?;
    let mut row = vec![None; n_targets];
    let mut remaining = target_cells.len();
    let stats = solve_until(
        reader,
        scenario,
        &[source],
        config,
        &mut state,
        stop,
        &mut |r, c, cell| {
            if let Some(targets) = target_cells.get(&(r * cols + c)) {
                for &t in targets {
                    row[t] = Some(Pair {
                        time_s: cell.time_s,
                        distance_m: cell.distance_m,
                    });
                }
                remaining -= 1;
                if remaining == 0 || first_only {
                    return Flow::Stop;
                }
            }
            Flow::Continue
        },
    )?;
    Ok((row, stats.settled))
}
