//! Toolkit blocks against the GRASS modules they replace, on the AccessMod 5
//! demo, exported by `scripts/oracle/scaling-up-demo.R`: thin line
//! rasterisation (`vector::Burn::Thin`, `v.to.rast` without `-d`) and the
//! distance to points (`focal::distance_to_sources`, `r.grow.distance`).
//! Skip, with a message, when the export is absent.

use accessmod_core::focal::distance_to_sources;
use accessmod_core::store::{TileGeometry, TiledGrid};
use accessmod_io::error::Error;
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

#[test]
fn demo_facility_distances_match_grass() {
    let root = oracle_dir("");
    let grass = root.join("scaling-up/demo/oracle/distance_facilities.tif");
    let facilities = root.join("travel-time/demo/input/facilities.fgb");
    if !oracle_available(&[&grass, &facilities]) {
        return;
    }
    let (_, grid) = GridSpec::open_dem(&root.join("travel-time/demo/input/dem.tif")).expect("dem");
    let layer = VectorLayer::open(&facilities, None, None, &grid, "facilities").expect("layer");
    let geometry = TileGeometry::new(grid.rows, grid.cols, 64).expect("geometry");
    let tmp = tempfile::tempdir().expect("tmp");
    let mut cells = TiledGrid::new(geometry, f64::INFINITY, 4, tmp.path(), "d").expect("grid");
    burn_layer(&layer, &grid, &mut cells, Burn::Thin, &|| false, |_| {
        Ok(Some(|c: &mut f64| *c = 0.0))
    })
    .expect("burn");
    distance_to_sources::<Error>(&mut cells, grid.cell.ew_m(), grid.cell.ns_m(), || Ok(()))
        .expect("distance");
    let grass = read_valid(&grass);
    let mut differ = 0;
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let ours = cells.get(r, c).expect("cell");
            if Some(ours) != grass[r * grid.cols + c] {
                differ += 1;
            }
        }
    }
    assert_eq!(differ, 0, "cells differ from r.grow.distance");
}
