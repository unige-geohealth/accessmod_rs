//! Synthetic solver run for timing and cache behaviour.
//!
//! `cargo run --release -p accessmod-core --example synthetic -- <side> <tile> <capacity> [16]`
//!
//! Builds a `side × side` anisotropic landscape (walking, bicycling and
//! motorized classes on a rolling DEM, a few barriers), solves it from a few
//! facilities and prints timings and cache counters.

use std::time::Instant;

use accessmod_core::cost::{CellMobility, CellSize, Direction, Neighbourhood};
use accessmod_core::mobility::{MobilityModel, TransportMode};
use accessmod_core::solver::{Facility, InputCell, Method, SolverCell, SolverConfig, solve};
use accessmod_core::store::{TileGeometry, TiledGrid};

fn arg(args: &[String], i: usize, default: usize) -> usize {
    args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let side = arg(&args, 0, 2000);
    let tile = arg(&args, 1, 256);
    let capacity = arg(&args, 2, usize::MAX);
    let neighbourhood = if arg(&args, 3, 8) == 16 {
        Neighbourhood::Sixteen
    } else {
        Neighbourhood::Eight
    };

    let geometry = TileGeometry::new(side, side, tile)?;
    let capacity = capacity.min(geometry.n_tiles());
    let dir = std::env::temp_dir();
    let scenario = [
        CellMobility::new(TransportMode::Walking, 5.0).ok_or("speed")?,
        CellMobility::new(TransportMode::Bicycling, 15.0).ok_or("speed")?,
        CellMobility::new(TransportMode::Motorized, 60.0).ok_or("speed")?,
    ];

    let start = Instant::now();
    let mut inputs = TiledGrid::new(geometry, InputCell::NODATA, capacity, &dir, "inputs")?;
    let mut row = vec![InputCell::NODATA; side];
    for r in 0..side {
        for (c, cell) in row.iter_mut().enumerate() {
            let (x, y) = (c as f64, r as f64);
            let dem_m = 300.0 * ((x / 97.0).sin() * (y / 131.0).cos()) + 0.01 * x;
            let slot = match (r * 7 + c * 13) % 50 {
                0 => InputCell::BARRIER,
                k if k < 30 => 0,
                k if k < 45 => 1,
                _ => 2,
            };
            *cell = InputCell { slot, dem_m };
        }
        for (c, cell) in row.iter().enumerate() {
            inputs.set(r, c, *cell)?;
        }
    }
    let build = start.elapsed();

    let mut state = TiledGrid::new(geometry, SolverCell::UNREACHED, capacity, &dir, "state")?;
    let facilities: Vec<Facility> = [(0.1, 0.1), (0.5, 0.5), (0.9, 0.2), (0.3, 0.8)]
        .iter()
        .map(|&(fr, fc)| Facility {
            row: (fr * side as f64) as usize,
            col: (fc * side as f64) as usize,
        })
        .collect();
    for f in &facilities {
        let mut input = inputs.get(f.row, f.col)?;
        input.slot = 0;
        inputs.set(f.row, f.col, input)?;
    }
    let config = SolverConfig {
        method: Method::Anisotropic(MobilityModel::Accessmod5V1),
        direction: Direction::FromFacilities,
        neighbourhood,
        cell: CellSize::new(100.0, 100.0).ok_or("cell size")?,
        limit_s: None,
    };
    let start = Instant::now();
    let stats = solve(
        &mut inputs,
        &scenario,
        &facilities,
        &config,
        &mut state,
        &|| false,
    )?;
    let solve_time = start.elapsed();

    let cells = side * side;
    let tile_mb = |bytes: usize| bytes as f64 * capacity as f64 / 1e6;
    println!(
        "grid {side}x{side} ({cells} cells), tile {tile}, capacity {capacity}/{} tiles, {} neighbours",
        geometry.n_tiles(),
        neighbourhood.count()
    );
    println!(
        "resident: inputs {:.0} MB, state {:.0} MB",
        tile_mb(TiledGrid::<InputCell>::tile_bytes(geometry)),
        tile_mb(TiledGrid::<SolverCell>::tile_bytes(geometry))
    );
    println!(
        "build {:.2?}, solve {:.2?} ({:.0} ns/cell)",
        build,
        solve_time,
        solve_time.as_nanos() as f64 / cells as f64
    );
    println!("solver: {stats:?}");
    println!("inputs cache: {:?}", inputs.stats());
    println!("state cache: {:?}", state.stats());
    Ok(())
}
