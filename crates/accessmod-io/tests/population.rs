//! Adjust population distribution on small fixtures built with GDAL.

use std::path::Path;

use accessmod_io::error::ErrorKind;
use accessmod_io::fixtures::{
    EPSG, X0, Y0, geo_transform, grid, never, read, run_job, settings, write_layer, write_raster,
};
use accessmod_io::job::SCHEMA_VERSION;
use accessmod_io::output::FLOAT_NODATA;
use accessmod_io::population::{
    AdjustPopulation, AdjustPopulationJob, POPULATION_FILE, ZONES_FILE, ZoneSource, prepare, run,
};
use gdal::vector::{FieldValue, OGRFieldType};

/// Polygon over cells `[c0, c1) × [r0, r1)` of the fixture grid.
fn cells(c0: f64, r0: f64, c1: f64, r1: f64) -> String {
    let g = grid(1, 1);
    let corners = [(c0, r0), (c1, r0), (c1, r1), (c0, r1), (c0, r0)];
    let points: Vec<String> = corners
        .iter()
        .map(|&(c, r)| {
            let (x, y) = g.to_map(c, r);
            format!("{x} {y}")
        })
        .collect();
    format!("POLYGON (({}))", points.join(", "))
}

/// 4 × 2 grid, 10 people per cell, land cover NoData at (0, 0).
/// Zone A: columns 0–1; zone B: (0, 2) and (0, 3); zone C: cell (0, 0),
/// inside A (A keeps it). Cells (1, 2) and (1, 3) are outside every zone.
fn fixture(dir: &Path, known: bool) -> AdjustPopulationJob {
    let (cols, rows) = (4, 2);
    let gt = geo_transform(X0, Y0);
    write_raster::<f32>(&dir.join("dem.tif"), cols, rows, gt, vec![0.0; 8], None);
    write_raster::<f32>(
        &dir.join("pop.tif"),
        cols,
        rows,
        gt,
        vec![10.0; 8],
        Some(-1.0),
    );
    let mut lc = vec![1u8; 8];
    lc[0] = 0;
    write_raster::<u8>(&dir.join("lc.tif"), cols, rows, gt, lc, Some(0.0));
    let zones = [
        ("A", 60.0, cells(0.0, 0.0, 2.0, 2.0)),
        ("B", 0.0, cells(2.0, 0.0, 4.0, 1.0)),
        ("C", 5.0, cells(0.0, 0.0, 1.0, 1.0)),
    ];
    let features: Vec<(Option<FieldValue>, &str)> = zones
        .iter()
        .map(|(id, _, wkt)| (Some(FieldValue::StringValue((*id).into())), wkt.as_str()))
        .collect();
    write_layer(
        &dir.join("zones.gpkg"),
        "zones",
        EPSG,
        ("name", OGRFieldType::OFTString),
        &features,
    );
    if known {
        // Same zones with a known population field.
        let wkts: Vec<(&str, Vec<Option<FieldValue>>)> = zones
            .iter()
            .map(|(id, pop, wkt)| {
                (
                    wkt.as_str(),
                    vec![
                        Some(FieldValue::StringValue((*id).into())),
                        Some(FieldValue::RealValue(*pop)),
                    ],
                )
            })
            .collect();
        std::fs::remove_file(dir.join("zones.gpkg")).expect("remove");
        accessmod_io::fixtures::write_features(
            &dir.join("zones.gpkg"),
            "zones",
            EPSG,
            &[
                ("name", OGRFieldType::OFTString),
                ("pop", OGRFieldType::OFTReal),
            ],
            &wkts,
        );
    }
    AdjustPopulationJob {
        schema_version: SCHEMA_VERSION,
        dem: dir.join("dem.tif"),
        population: dir.join("pop.tif"),
        land_cover: dir.join("lc.tif"),
        zones: ZoneSource {
            path: dir.join("zones.gpkg"),
            layer: None,
            id_field: "name".into(),
            filter: None,
            population_field: known.then(|| "pop".into()),
        },
        output_dir: dir.join("out"),
    }
}

#[test]
fn barrier_population_moves_within_its_zone() {
    let dir = tempfile::tempdir().expect("tmp");
    let job = fixture(dir.path(), false);
    let prepared = prepare(job, &settings(256 << 20, 1), &never).expect("prepare");
    let (out, report) = run(prepared, &settings(256 << 20, 1), &never).expect("run");
    let n = FLOAT_NODATA;
    // 40 people, 30 off barriers: ratio 4/3 on each 10-people cell.
    let a = 10.0 * (40.0 / 30.0);
    assert_eq!(
        read::<f64>(&out.join(POPULATION_FILE)),
        vec![n, a, 10.0, 10.0, a, a, n, n]
    );
    let r = &report.run.result;
    assert_eq!(r.population, 80.0);
    assert_eq!(r.population_on_barrier, 10.0);
    assert_eq!(r.population_outside_zones, 20.0);
    assert!((r.population_adjusted - 60.0).abs() < 1e-12);
    assert_eq!(r.overlapping_cells, 1);
    assert_eq!(r.zones_without_population, 1);
    let csv = std::fs::read_to_string(out.join(ZONES_FILE)).expect("zones.csv");
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[2], "B,2,20,0,,1,20");
    assert!(lines[3].starts_with("C,0,0,0,,,0"), "{}", lines[3]);
}

#[test]
fn known_populations_are_reached() {
    let dir = tempfile::tempdir().expect("tmp");
    let job = fixture(dir.path(), true);
    let out = run_job::<AdjustPopulation>(job, &settings(256 << 20, 1)).expect("run");
    let n = FLOAT_NODATA;
    assert_eq!(
        read::<f64>(&out.join(POPULATION_FILE)),
        vec![n, 20.0, 0.0, 0.0, 20.0, 20.0, n, n]
    );
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("run.json")).expect("run.json"))
            .expect("json");
    // Zone C has no cell: its 5 people cannot be placed.
    assert_eq!(report["run"]["result"]["population_unplaced"], 5.0);
}

#[test]
fn zone_problems_are_validation_errors() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = fixture(dir.path(), false);
    job.zones.population_field = Some("missing".into());
    let err = prepare(job.clone(), &settings(256 << 20, 1), &never).expect_err("no field");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.to_string().contains("no field \"missing\""), "{err}");
    job.zones.population_field = None;
    job.zones.id_field = "fid_missing".into();
    assert!(prepare(job, &settings(256 << 20, 1), &never).is_err());
}

#[test]
fn outputs_do_not_depend_on_memory() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path();
    let (cols, rows) = (900, 700);
    let gt = geo_transform(X0, Y0);
    write_raster::<f32>(
        &path.join("dem.tif"),
        cols,
        rows,
        gt,
        vec![0.0; cols * rows],
        None,
    );
    let pop: Vec<f32> = (0..cols * rows).map(|i| (i % 97) as f32 * 0.37).collect();
    write_raster::<f32>(&path.join("pop.tif"), cols, rows, gt, pop, Some(-1.0));
    let lc: Vec<u8> = (0..cols * rows)
        .map(|i| u8::from((i / cols + i % cols) % 13 != 0))
        .collect();
    write_raster::<u8>(&path.join("lc.tif"), cols, rows, gt, lc, Some(0.0));
    let zones: Vec<String> = (0..6)
        .map(|k| {
            let c0 = (k % 3) as f64 * 300.0 + 0.3;
            let r0 = (k / 3) as f64 * 350.0 + 0.6;
            cells(c0, r0, c0 + 310.0, r0 + 340.0)
        })
        .collect();
    let features: Vec<(Option<FieldValue>, &str)> = zones
        .iter()
        .enumerate()
        .map(|(k, wkt)| (Some(FieldValue::Integer64Value(k as i64)), wkt.as_str()))
        .collect();
    write_layer(
        &path.join("zones.gpkg"),
        "zones",
        EPSG,
        ("zone", OGRFieldType::OFTInteger64),
        &features,
    );
    let job = |out: &str| AdjustPopulationJob {
        schema_version: SCHEMA_VERSION,
        dem: path.join("dem.tif"),
        population: path.join("pop.tif"),
        land_cover: path.join("lc.tif"),
        zones: ZoneSource {
            path: path.join("zones.gpkg"),
            layer: None,
            id_field: "zone".into(),
            filter: None,
            population_field: None,
        },
        output_dir: path.join(out),
    };
    // 20 MiB: the 16 MiB GDAL cache floor and buffers leave a fraction of
    // the 165 zone tiles resident.
    let tight = prepare(job("a"), &settings(20 << 20, 1), &never).expect("prepare");
    assert!(tight.plan.resident_tiles < tight.plan.total_tiles);
    let (out_a, report_a) = run(tight, &settings(20 << 20, 1), &never).expect("run a");
    assert!(
        report_a.execution.details.store.tile_writes > 0,
        "scratch not exercised"
    );
    let out_b = run_job::<AdjustPopulation>(job("b"), &settings(4 << 30, 4)).expect("run b");
    for name in [POPULATION_FILE, ZONES_FILE] {
        let a = std::fs::read(out_a.join(name)).expect("read a");
        let b = std::fs::read(out_b.join(name)).expect("read b");
        assert!(a == b, "{name} differs between budgets");
    }
    assert!(report_a.run.result.overlapping_cells > 0);
}
