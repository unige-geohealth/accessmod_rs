//! Thin line rasterisation (`vector::Burn::Thin`) against GRASS `v.to.rast`
//! without `-d` on the AccessMod 5 demo roads, exported by
//! `scripts/oracle/scaling-up-demo.R`. Skips, with a message, when the
//! export is absent.

use accessmod_core::store::{TileGeometry, TiledGrid};
use accessmod_io::fixtures::{oracle_available, oracle_dir, read_valid};
use accessmod_io::grid::GridSpec;
use accessmod_io::vector::{Burn, VectorLayer, burn_layer};

#[test]
fn demo_roads_match_grass_thin_lines() {
    let root = oracle_dir("");
    let grass = root.join("scaling-up/demo/oracle/roads_thin.tif");
    let roads = root.join("merge-land-cover/demo/input/roads.fgb");
    if !oracle_available(&[&grass, &roads]) {
        return;
    }
    let (_, grid) = GridSpec::open_dem(&root.join("travel-time/demo/input/dem.tif")).expect("dem");
    let layer = VectorLayer::open(&roads, None, None, &grid, "roads").expect("roads");
    let geometry = TileGeometry::new(grid.rows, grid.cols, 64).expect("geometry");
    let tmp = tempfile::tempdir().expect("tmp");
    let mut cells = TiledGrid::new(geometry, 0u8, 4, tmp.path(), "roads").expect("grid");
    burn_layer(&layer, &grid, &mut cells, Burn::Thin, &|| false, |_| {
        Ok(Some(|c: &mut u8| *c = 1))
    })
    .expect("burn");
    let grass = read_valid(&grass);
    let mut differ = Vec::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let ours = cells.get(r, c).expect("cell") == 1;
            if ours != grass[r * grid.cols + c].is_some() {
                differ.push((r, c));
            }
        }
    }
    assert!(
        differ.is_empty(),
        "{} cells differ: {:?}",
        differ.len(),
        &differ[..differ.len().min(10)]
    );
}
