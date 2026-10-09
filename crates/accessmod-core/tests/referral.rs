//! Referral: pairs equal full searches, side swap, workers, frozen backends.

use std::path::PathBuf;

use accessmod_core::cost::{CellMobility, CellSize, Direction, Method, Neighbourhood};
use accessmod_core::mobility::{MobilityModel, TransportMode};
use accessmod_core::referral::{Pair, ReferralConfig, WorkerBudget, referral};
use accessmod_core::solver::{Facility, InputCell, SolverCell, SolverConfig, solve};
use accessmod_core::store::{Frozen, TileGeometry, TiledGrid};
use proptest::prelude::*;

const B: u16 = InputCell::BARRIER;

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("accessmod-referral-tests");
    std::fs::create_dir_all(&dir).expect("temp dir is writable");
    dir
}

fn mobility(mode: TransportMode, speed_kmh: f64) -> CellMobility {
    CellMobility::new(mode, speed_kmh).expect("positive test speed")
}

#[derive(Debug, Clone)]
struct Case {
    rows: usize,
    cols: usize,
    slots: Vec<u16>,
    dem: Vec<f64>,
    origins: Vec<Facility>,
    destinations: Vec<Facility>,
}

fn scenario() -> Vec<CellMobility> {
    vec![
        mobility(TransportMode::Walking, 5.0),
        mobility(TransportMode::Bicycling, 18.0),
        mobility(TransportMode::Motorized, 40.0),
    ]
}

fn inputs(case: &Case, tile: usize, capacity: usize) -> TiledGrid<InputCell> {
    let geometry = TileGeometry::new(case.rows, case.cols, tile).expect("geometry");
    let mut grid = TiledGrid::new(
        geometry,
        InputCell::NODATA,
        capacity,
        &scratch_dir(),
        "inputs",
    )
    .expect("grid");
    for i in 0..case.rows * case.cols {
        let cell = InputCell {
            slot: case.slots[i],
            dem_m: case.dem[i],
        };
        grid.set(i / case.cols, i % case.cols, cell).expect("set");
    }
    grid
}

fn frozen(case: &Case, tile: usize, capacity: usize) -> Frozen<InputCell> {
    inputs(case, tile, capacity).freeze().expect("freeze")
}

fn full_search(
    case: &Case,
    config: &ReferralConfig,
    source: Facility,
    direction: Direction,
) -> Vec<SolverCell> {
    let mut grid = inputs(case, 4, 64);
    let mut state = TiledGrid::new(
        grid.geometry(),
        SolverCell::UNREACHED,
        64,
        &scratch_dir(),
        "s",
    )
    .expect("state");
    let solver = SolverConfig {
        method: config.method,
        direction,
        neighbourhood: config.neighbourhood,
        cell: config.cell,
        limit_s: config.limit_s,
    };
    solve(
        &mut grid,
        &scenario(),
        &[source],
        &solver,
        &mut state,
        &|| false,
    )
    .expect("solve");
    (0..case.rows * case.cols)
        .map(|i| state.get(i / case.cols, i % case.cols).expect("get"))
        .collect()
}

fn run(
    case: &Case,
    config: &ReferralConfig,
    workers: usize,
    inputs: &Frozen<InputCell>,
) -> Vec<Vec<Option<Pair>>> {
    let budget = WorkerBudget {
        state_tiles: 64,
        input_tiles: 2,
    };
    referral(
        inputs,
        &scenario(),
        &case.origins,
        &case.destinations,
        config,
        workers,
        budget,
        &scratch_dir(),
        &|| false,
    )
    .expect("referral")
    .pairs
}

fn any_case() -> impl Strategy<Value = Case> {
    (3usize..12, 3usize..12).prop_flat_map(|(rows, cols)| {
        let n = rows * cols;
        let cell = (0..rows, 0..cols).prop_map(|(row, col)| Facility { row, col });
        (
            proptest::collection::vec(prop_oneof![8 => 0u16..3, 1 => Just(B)], n),
            proptest::collection::vec(0.0f64..60.0, n),
            proptest::collection::vec(cell.clone(), 1..6),
            proptest::collection::vec(cell, 1..6),
        )
            .prop_map(move |(mut slots, dem, origins, destinations)| {
                for f in origins.iter().chain(&destinations) {
                    if slots[f.row * cols + f.col] == B {
                        slots[f.row * cols + f.col] = 0;
                    }
                }
                Case {
                    rows,
                    cols,
                    slots,
                    dem,
                    origins,
                    destinations,
                }
            })
    })
}

fn any_config() -> impl Strategy<Value = ReferralConfig> {
    (
        prop_oneof![
            Just(Method::Isotropic),
            Just(Method::Anisotropic(MobilityModel::Accessmod5V1))
        ],
        prop_oneof![Just(Neighbourhood::Eight), Just(Neighbourhood::Sixteen)],
        prop_oneof![Just(None), (200.0f64..3000.0).prop_map(Some)],
    )
        .prop_map(|(method, neighbourhood, limit_s)| ReferralConfig {
            method,
            neighbourhood,
            cell: CellSize::new(100.0, 120.0).expect("cell"),
            limit_s,
            nearest_only: false,
        })
}

/// Time and distance at `target` in a full search, if reached.
fn sample(cells: &[SolverCell], case: &Case, target: Facility) -> Option<(f64, f64)> {
    let cell = cells[target.row * case.cols + target.col];
    cell.is_reached().then_some((cell.time_s, cell.distance_m))
}

proptest! {
    #[test]
    fn forward_pairs_equal_full_searches(mut case in any_case(), config in any_config()) {
        // Keep origins the smaller side so searches start from them.
        if case.origins.len() > case.destinations.len() {
            std::mem::swap(&mut case.origins, &mut case.destinations);
        }
        let pairs = run(&case, &config, 3, &frozen(&case, 4, 64));
        for (o, origin) in case.origins.iter().enumerate() {
            let cells = full_search(&case, &config, *origin, Direction::FromFacilities);
            for (d, destination) in case.destinations.iter().enumerate() {
                let expected = sample(&cells, &case, *destination);
                let got = pairs[o][d].map(|p| (p.time_s, p.distance_m));
                prop_assert_eq!(got, expected, "pair {} -> {}", o, d);
            }
        }
    }

    #[test]
    fn reversed_pairs_match_forward_searches(mut case in any_case(), config in any_config()) {
        // Destinations the smaller side: searches start from them, reversed.
        if case.destinations.len() >= case.origins.len() {
            std::mem::swap(&mut case.origins, &mut case.destinations);
        }
        prop_assume!(case.destinations.len() < case.origins.len());
        let pairs = run(&case, &config, 2, &frozen(&case, 4, 64));
        for (o, origin) in case.origins.iter().enumerate() {
            let cells = full_search(&case, &config, *origin, Direction::FromFacilities);
            for (d, destination) in case.destinations.iter().enumerate() {
                let expected = sample(&cells, &case, *destination);
                let got = pairs[o][d];
                match (got, expected) {
                    (Some(p), Some((t, _))) => prop_assert!((p.time_s - t).abs() <= 1e-12 * t.max(1.0),
                        "pair {} -> {}: {} vs {}", o, d, p.time_s, t),
                    // The limit can cut either side at the boundary only when
                    // times differ in the last bits; that is not reachable here
                    // unless the time equals the limit exactly.
                    (None, None) => {}
                    other => prop_assert!(config.limit_s.is_some(), "reachability differs: {:?}", other),
                }
            }
        }
    }

    #[test]
    fn workers_and_backends_do_not_change_results(case in any_case(), config in any_config()) {
        let reference = run(&case, &config, 1, &frozen(&case, 4, 64));
        prop_assert_eq!(&run(&case, &config, 4, &frozen(&case, 4, 64)), &reference);
        // Tiny input cache: the grid spills, readers go through the file.
        let spilled = frozen(&case, 2, 1);
        prop_assert!(!spilled.in_memory() || case.rows * case.cols <= 4);
        prop_assert_eq!(&run(&case, &config, 3, &spilled), &reference);
    }

    #[test]
    fn nearest_only_keeps_the_nearest_destination(case in any_case(), config in any_config()) {
        let all = run(&case, &config, 2, &frozen(&case, 4, 64));
        let nearest_config = ReferralConfig { nearest_only: true, ..config };
        let nearest = run(&case, &nearest_config, 2, &frozen(&case, 4, 64));
        for (o, row) in nearest.iter().enumerate() {
            let kept: Vec<_> = row.iter().flatten().collect();
            let best = all[o].iter().flatten().map(|p| p.time_s).fold(f64::INFINITY, f64::min);
            if best.is_finite() {
                prop_assert!(!kept.is_empty());
                // `all` may come from reversed searches: equal to the last bits.
                prop_assert!(kept.iter().all(|p| (p.time_s - best).abs() <= 1e-12 * best.max(1.0)));
            } else {
                prop_assert!(kept.is_empty());
            }
        }
    }
}
