//! Helpers shared by the integration tests of `accessmod-core`.

use std::path::PathBuf;

use accessmod_core::cost::CellMobility;
use accessmod_core::mobility::TransportMode;
use accessmod_core::solver::InputCell;

/// Barrier slot, short for test grids.
pub const B: u16 = InputCell::BARRIER;

/// Scratch directory `name` under the system temporary directory.
pub fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&dir).expect("temp dir is writable");
    dir
}

/// Mobility of a valid test speed.
pub fn mobility(mode: TransportMode, speed_kmh: f64) -> CellMobility {
    CellMobility::new(mode, speed_kmh).expect("positive test speed")
}
