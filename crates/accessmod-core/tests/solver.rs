//! Solver behaviour on small hand-computable grids, storage invariance and an
//! independent Bellman-Ford reference.

use std::path::PathBuf;

use accessmod_core::cost::{CellMobility, CellSize, Direction, Edge, Neighbourhood};
use accessmod_core::mobility::{MobilityModel, TransportMode};
use accessmod_core::solver::{
    Facility, InputCell, Method, SolveError, SolverCell, SolverConfig, solve,
};
use accessmod_core::store::{TileGeometry, TiledGrid};
use proptest::prelude::*;

const B: u16 = InputCell::BARRIER;

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("accessmod-solver-tests");
    std::fs::create_dir_all(&dir).expect("temp dir is writable");
    dir
}

/// Grid described row-major by scenario slots and elevations.
#[derive(Debug)]
struct Case {
    rows: usize,
    cols: usize,
    slots: Vec<u16>,
    dem: Vec<f64>,
    scenario: Vec<CellMobility>,
}

impl Case {
    fn flat(rows: usize, cols: usize, slots: Vec<u16>, scenario: Vec<CellMobility>) -> Self {
        Self {
            rows,
            cols,
            dem: vec![0.0; slots.len()],
            slots,
            scenario,
        }
    }

    fn run(
        &self,
        facilities: &[(usize, usize)],
        config: &SolverConfig,
        tile: usize,
        capacity: usize,
    ) -> Result<Vec<SolverCell>, SolveError> {
        let geometry = TileGeometry::new(self.rows, self.cols, tile).expect("valid geometry");
        let dir = scratch_dir();
        let mut inputs =
            TiledGrid::new(geometry, InputCell::NODATA, capacity, &dir, "inputs").expect("grid");
        for (i, (&slot, &dem_m)) in self.slots.iter().zip(&self.dem).enumerate() {
            inputs
                .set(i / self.cols, i % self.cols, InputCell { slot, dem_m })
                .expect("set");
        }
        let mut state =
            TiledGrid::new(geometry, SolverCell::UNREACHED, capacity, &dir, "state").expect("grid");
        let facilities: Vec<Facility> = facilities
            .iter()
            .map(|&(row, col)| Facility { row, col })
            .collect();
        solve(
            &mut inputs,
            &self.scenario,
            &facilities,
            config,
            &mut state,
            &|| false,
        )?;
        let mut out = Vec::with_capacity(self.rows * self.cols);
        for r in 0..self.rows {
            for c in 0..self.cols {
                out.push(state.get(r, c).expect("get"));
            }
        }
        Ok(out)
    }
}

fn mobility(mode: TransportMode, speed_kmh: f64) -> CellMobility {
    CellMobility::new(mode, speed_kmh).expect("positive test speed")
}

fn motorized(speed_kmh: f64) -> CellMobility {
    mobility(TransportMode::Motorized, speed_kmh)
}

fn config(method: Method, neighbourhood: Neighbourhood) -> SolverConfig {
    SolverConfig {
        method,
        direction: Direction::FromFacilities,
        neighbourhood,
        cell: CellSize::new(100.0, 100.0).expect("valid cell size"),
        limit_s: None,
    }
}

const ISO8: fn() -> SolverConfig = || config(Method::Isotropic, Neighbourhood::Eight);

#[test]
fn corridor_by_hand() {
    // 100 m at 5 km/h = 72 s per step.
    let case = Case::flat(1, 5, vec![0; 5], vec![motorized(5.0)]);
    let out = case.run(&[(0, 0)], &ISO8(), 2, 1).expect("solve");
    for (c, cell) in out.iter().enumerate() {
        assert_eq!(cell.time_s, 72.0 * c as f64);
        assert_eq!(cell.distance_m, 100.0 * c as f64);
        assert_eq!(cell.nearest, 1);
        assert_eq!(cell.step, if c == 0 { 0 } else { 2 }, "E has code 2");
    }
}

#[test]
fn equal_times_go_to_the_first_facility_in_layer_order() {
    let case = Case::flat(1, 5, vec![0; 5], vec![motorized(5.0)]);
    let out = case.run(&[(0, 0), (0, 4)], &ISO8(), 2, 1).expect("solve");
    assert_eq!(out[2].nearest, 1);
    let out = case.run(&[(0, 4), (0, 0)], &ISO8(), 2, 1).expect("solve");
    assert_eq!(
        out[2].nearest, 1,
        "first listed is now the eastern facility"
    );
    assert_eq!(out[2].time_s, 144.0);
}

#[test]
fn shared_cells_keep_the_first_facility() {
    let case = Case::flat(1, 3, vec![0; 3], vec![motorized(5.0)]);
    let out = case.run(&[(0, 1), (0, 1)], &ISO8(), 2, 1).expect("solve");
    assert!(out.iter().all(|c| c.nearest == 1));
}

#[test]
fn barriers_are_walked_around() {
    // 3x3, centre barrier: (0,0) -> (2,2) needs two diagonal-free detours.
    let mut slots = vec![0; 9];
    slots[4] = B;
    let case = Case::flat(3, 3, slots, vec![motorized(5.0)]);
    let out = case.run(&[(0, 0)], &ISO8(), 2, 1).expect("solve");
    assert!(!out[4].is_reached());
    // (0,0) -> (0,1) -> (1,2) -> (2,2): 100 + 141.42 + 100 m.
    let expected = 100.0 + 2f64.sqrt() * 100.0 + 100.0;
    assert!(
        (out[8].distance_m - expected).abs() < 1e-9,
        "{}",
        out[8].distance_m
    );
}

#[test]
fn knight_steps_never_cross_a_barrier() {
    // (0,0) -> (1,2) is ESE (code 15), crossing E (0,1) and SE (1,1).
    let mut slots = vec![0; 6];
    slots[1] = B;
    let case = Case::flat(2, 3, slots, vec![motorized(5.0)]);
    let config16 = config(Method::Isotropic, Neighbourhood::Sixteen);
    let out = case.run(&[(0, 0)], &config16, 2, 1).expect("solve");
    assert!(out[5].is_reached());
    assert_ne!(out[5].step, 15);

    let open = Case::flat(2, 3, vec![0; 6], vec![motorized(5.0)]);
    let out = open.run(&[(0, 0)], &config16, 2, 1).expect("solve");
    assert_eq!(out[5].step, 15, "without the barrier the knight step wins");
}

#[test]
fn limit_is_inclusive() {
    let case = Case::flat(1, 5, vec![0; 5], vec![motorized(5.0)]);
    let mut limited = ISO8();
    limited.limit_s = Some(144.0);
    let out = case.run(&[(0, 0)], &limited, 2, 1).expect("solve");
    assert!(out[2].is_reached() && !out[3].is_reached());
    limited.limit_s = Some(143.999);
    let out = case.run(&[(0, 0)], &limited, 2, 1).expect("solve");
    assert!(out[1].is_reached() && !out[2].is_reached());
}

#[test]
fn nodata_dem_is_a_barrier() {
    let mut case = Case::flat(1, 3, vec![0; 3], vec![motorized(5.0)]);
    case.dem[1] = f64::NAN;
    let out = case.run(&[(0, 0)], &ISO8(), 2, 1).expect("solve");
    assert!(!out[1].is_reached() && !out[2].is_reached());
}

#[test]
fn facility_on_a_barrier_is_an_error() {
    let case = Case::flat(1, 3, vec![0, B, 0], vec![motorized(5.0)]);
    let err = case.run(&[(0, 1)], &ISO8(), 2, 1).expect_err("barrier");
    assert!(
        matches!(err, SolveError::Facility { ordinal: 1, .. }),
        "{err}"
    );
    let err = case.run(&[(0, 9)], &ISO8(), 2, 1).expect_err("outside");
    assert!(matches!(err, SolveError::Facility { .. }), "{err}");
}

#[test]
fn towards_facilities_inverts_slopes() {
    // Facility at the bottom of a 10 % slope; walking.
    let walking = mobility(TransportMode::Walking, 5.0);
    let mut case = Case::flat(1, 2, vec![0; 2], vec![walking]);
    case.dem = vec![0.0, 10.0];
    let mut aniso = config(
        Method::Anisotropic(MobilityModel::Accessmod5V1),
        Neighbourhood::Eight,
    );
    let up = case.run(&[(0, 0)], &aniso, 2, 1).expect("solve")[1].time_s;
    aniso.direction = Direction::TowardsFacilities;
    let down = case.run(&[(0, 0)], &aniso, 2, 1).expect("solve")[1].time_s;
    assert!(
        down < up,
        "walking down to the facility is faster: {down} vs {up}"
    );
}

#[test]
fn cancellation_is_honoured() {
    let n = 100;
    let case = Case::flat(n, n, vec![0; n * n], vec![motorized(5.0)]);
    let geometry = TileGeometry::new(n, n, 16).expect("valid");
    let dir = scratch_dir();
    let mut inputs = TiledGrid::new(geometry, InputCell::NODATA, 64, &dir, "i").expect("grid");
    for i in 0..n * n {
        inputs
            .set(
                i / n,
                i % n,
                InputCell {
                    slot: case.slots[i],
                    dem_m: 0.0,
                },
            )
            .expect("set");
    }
    let mut state = TiledGrid::new(geometry, SolverCell::UNREACHED, 64, &dir, "s").expect("grid");
    let result = solve(
        &mut inputs,
        &case.scenario,
        &[Facility { row: 0, col: 0 }],
        &ISO8(),
        &mut state,
        &|| true,
    );
    assert!(matches!(result, Err(SolveError::Cancelled)));
}

/// Random landscape: slots, elevations, barriers.
fn any_case() -> impl Strategy<Value = (Case, Vec<(usize, usize)>)> {
    (2usize..12, 2usize..12).prop_flat_map(|(rows, cols)| {
        let n = rows * cols;
        (
            proptest::collection::vec(prop_oneof![8 => 0u16..3, 1 => Just(B)], n),
            proptest::collection::vec(0.0f64..50.0, n),
            proptest::collection::vec((0..rows, 0..cols), 1..4),
        )
            .prop_map(move |(slots, dem, facilities)| {
                let scenario = vec![
                    mobility(TransportMode::Walking, 5.0),
                    mobility(TransportMode::Bicycling, 18.0),
                    motorized(40.0),
                ];
                let mut slots = slots;
                for &(r, c) in &facilities {
                    if slots[r * cols + c] == B {
                        slots[r * cols + c] = 0;
                    }
                }
                (
                    Case {
                        rows,
                        cols,
                        slots,
                        dem,
                        scenario,
                    },
                    facilities,
                )
            })
    })
}

fn any_config() -> impl Strategy<Value = SolverConfig> {
    (
        prop_oneof![
            Just(Method::Isotropic),
            Just(Method::Anisotropic(MobilityModel::Accessmod5V1))
        ],
        prop_oneof![Just(Neighbourhood::Eight), Just(Neighbourhood::Sixteen)],
        prop_oneof![
            Just(Direction::FromFacilities),
            Just(Direction::TowardsFacilities)
        ],
        prop_oneof![Just(None), (100.0f64..2000.0).prop_map(Some)],
    )
        .prop_map(|(method, neighbourhood, direction, limit_s)| SolverConfig {
            method,
            direction,
            neighbourhood,
            cell: CellSize::new(90.0, 110.0).expect("valid cell size"),
            limit_s,
        })
}

/// Independent reference: Bellman-Ford relaxation of every edge until no
/// time improves, on plain vectors.
fn reference_times(case: &Case, facilities: &[(usize, usize)], config: &SolverConfig) -> Vec<f64> {
    let (rows, cols) = (case.rows, case.cols);
    let passable = |i: usize| case.slots[i] != B && case.dem[i].is_finite();
    let cell_mobility = |i: usize| passable(i).then(|| case.scenario[usize::from(case.slots[i])]);
    let mut time = vec![f64::INFINITY; rows * cols];
    for &(r, c) in facilities {
        time[r * cols + c] = 0.0;
    }
    loop {
        let mut changed = false;
        for o in 0..rows * cols {
            if !time[o].is_finite() {
                continue;
            }
            let (r, c) = ((o / cols) as isize, (o % cols) as isize);
            for &step in config.neighbourhood.steps() {
                let (tr, tc) = (r + isize::from(step.d_row), c + isize::from(step.d_col));
                if tr < 0 || tc < 0 || tr >= rows as isize || tc >= cols as isize {
                    continue;
                }
                let t = tr as usize * cols + tc as usize;
                if !passable(t) {
                    continue;
                }
                let origin = cell_mobility(o).expect("reached cells are passable");
                let edge = Edge::build(step, origin, |s| {
                    let i = (r + isize::from(s.d_row)) as usize * cols
                        + (c + isize::from(s.d_col)) as usize;
                    Ok::<_, ()>(cell_mobility(i))
                })
                .expect("infallible sampling");
                let Some(edge) = edge else { continue };
                let dz = case.dem[t] - case.dem[o];
                let Some(cost) = edge.cost_s(config.method, config.cell, dz, config.direction)
                else {
                    continue;
                };
                let candidate = time[o] + cost;
                if config.limit_s.is_some_and(|l| candidate > l) {
                    continue;
                }
                if candidate < time[t] {
                    time[t] = candidate;
                    changed = true;
                }
            }
        }
        if !changed {
            return time;
        }
    }
}

fn bits(cells: &[SolverCell]) -> Vec<(u64, u64, u32, u8)> {
    cells
        .iter()
        .map(|c| {
            (
                c.time_s.to_bits(),
                c.distance_m.to_bits(),
                c.nearest,
                c.step,
            )
        })
        .collect()
}

proptest! {
    #[test]
    fn matches_bellman_ford((case, facilities) in any_case(), config in any_config()) {
        let out = case.run(&facilities, &config, 4, 64).expect("solve");
        let reference = reference_times(&case, &facilities, &config);
        for (i, (cell, expected)) in out.iter().zip(&reference).enumerate() {
            prop_assert_eq!(cell.is_reached(), expected.is_finite(), "cell {}", i);
            if expected.is_finite() {
                prop_assert!((cell.time_s - expected).abs() <= 1e-9 * expected.max(1.0),
                    "cell {}: {} vs {}", i, cell.time_s, expected);
            }
        }
    }

    #[test]
    fn storage_never_changes_results((case, facilities) in any_case(), config in any_config()) {
        let in_memory = case.run(&facilities, &config, 64, 1).expect("solve");
        let spilled = case.run(&facilities, &config, 2, 1).expect("solve");
        let roomy = case.run(&facilities, &config, 3, 5).expect("solve");
        prop_assert_eq!(bits(&in_memory), bits(&spilled));
        prop_assert_eq!(bits(&in_memory), bits(&roomy));
    }

    #[test]
    fn times_do_not_depend_on_facility_order((case, facilities) in any_case(), config in any_config()) {
        let forward = case.run(&facilities, &config, 4, 64).expect("solve");
        let reversed: Vec<_> = facilities.iter().rev().copied().collect();
        let backward = case.run(&reversed, &config, 4, 64).expect("solve");
        for (a, b) in forward.iter().zip(&backward) {
            prop_assert_eq!(a.time_s.to_bits(), b.time_s.to_bits());
        }
    }
}
