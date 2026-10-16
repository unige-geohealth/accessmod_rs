//! End-to-end land-cover merges on small GeoTIFF / GeoPackage fixtures built
//! with GDAL in a temporary directory.

use std::path::{Path, PathBuf};

use accessmod_io::error::{Error, ErrorKind};
use accessmod_io::fixtures::{
    EPSG, X0, Y0, geo_transform, grid, never, read, run_job, settings, to_wgs84, write_layer,
    write_raster,
};
use accessmod_io::job::SCHEMA_VERSION;
use accessmod_io::merge::{
    BRIDGES_FILE, BarrierItem, CLASSES_FILE, LandCoverItem, MERGED_FILE, MERGED_NODATA,
    MergeLandCover, MergeLandCoverJob, Reclass, RoadItem, StackItem, prepare, run,
};
use gdal::vector::{FieldValue, OGRFieldType};

/// Map x, y of a point given in cell units (column, row from the top left).
fn map(col: f64, row: f64) -> (f64, f64) {
    grid(1, 1).to_map(col, row)
}

/// Replace every "col row" pair of a WKT in cell units by map coordinates.
fn convert_wkt(wkt: &str) -> String {
    let mut out = String::new();
    let mut number = String::new();
    let mut pending: Option<f64> = None;
    let flush = |number: &mut String, pending: &mut Option<f64>, out: &mut String| {
        if number.is_empty() {
            return;
        }
        let value: f64 = number.parse().expect("number");
        number.clear();
        match pending.take() {
            None => *pending = Some(value),
            Some(col) => {
                let (x, y) = map(col, value);
                out.push_str(&format!("{x} {y}"));
            }
        }
    };
    for c in wkt.chars() {
        if c.is_ascii_digit() || c == '.' || c == '-' {
            number.push(c);
        } else {
            flush(&mut number, &mut pending, &mut out);
            if pending.is_none() || c != ' ' {
                out.push(c);
            }
        }
    }
    flush(&mut number, &mut pending, &mut out);
    out
}

/// A layer `items` of WKT geometries in cell units (see [`map`]) with an
/// integer `class` field.
fn write_items(path: &Path, features: &[(Option<i64>, &str)]) {
    let wkts: Vec<String> = features.iter().map(|(_, wkt)| convert_wkt(wkt)).collect();
    let features: Vec<(Option<FieldValue>, &str)> = features
        .iter()
        .zip(&wkts)
        .map(|((class, _), wkt)| (class.map(FieldValue::Integer64Value), wkt.as_str()))
        .collect();
    write_layer(
        path,
        "items",
        EPSG,
        ("class", OGRFieldType::OFTInteger64),
        &features,
    );
}

fn merge(job: MergeLandCoverJob, memory_bytes: u64) -> Result<PathBuf, Error> {
    run_job::<MergeLandCover>(job, &settings(memory_bytes, 2))
}

fn road(path: PathBuf) -> StackItem {
    StackItem::Road(RoadItem {
        path,
        layer: None,
        class_field: "class".into(),
        label_field: None,
        filter: None,
        classes: None,
        add_1000: true,
    })
}

fn land_cover(path: PathBuf) -> StackItem {
    StackItem::LandCover(LandCoverItem {
        path,
        labels: Vec::new(),
        reclass: Vec::new(),
    })
}

fn barrier(path: PathBuf) -> StackItem {
    StackItem::Barrier(BarrierItem {
        path,
        layer: None,
        filter: None,
    })
}

/// 6 × 5 grid. Land cover: class 1, class 2 in column 5, NoData at (1, 2)
/// and (2, 3) so that the road cell (2, 2) only touches (1, 3) by a corner.
fn by_hand_fixture(dir: &Path) -> MergeLandCoverJob {
    let (cols, rows) = (6, 5);
    write_raster::<f32>(
        &dir.join("dem.tif"),
        cols,
        rows,
        geo_transform(X0, Y0),
        vec![0.0; cols * rows],
        Some(-9999.0),
    );
    let mut lc = vec![1u8; cols * rows];
    for r in 0..rows {
        lc[r * cols + 5] = 2;
    }
    lc[cols + 2] = 0;
    lc[2 * cols + 3] = 0;
    write_raster::<u8>(
        &dir.join("lc.tif"),
        cols,
        rows,
        geo_transform(X0, Y0),
        lc,
        Some(0.0),
    );
    // Road class 3 along row 4 (cells (4, 0) .. (4, 2)), class 7 inside
    // cell (2, 2) only, and a feature without class.
    write_items(
        &dir.join("roads.gpkg"),
        &[
            (Some(3), "LINESTRING (0.5 4.5, 2.5 4.5)"),
            (Some(7), "LINESTRING (2.2 2.5, 2.8 2.5)"),
            (None, "LINESTRING (0.5 0.5, 5.5 0.5)"),
        ],
    );
    // Barrier polygon over column 4 (centres x = 4.5), rows 0..=1.
    write_items(
        &dir.join("barrier.gpkg"),
        &[(
            None,
            "POLYGON ((4.1 0.1, 4.9 0.1, 4.9 1.9, 4.1 1.9, 4.1 0.1))",
        )],
    );
    MergeLandCoverJob {
        schema_version: SCHEMA_VERSION,
        dem: dir.join("dem.tif"),
        stack: vec![
            road(dir.join("roads.gpkg")),
            land_cover(dir.join("lc.tif")),
            barrier(dir.join("barrier.gpkg")),
        ],
        clean_bridges: true,
        output_dir: dir.join("out"),
    }
}

#[test]
fn stack_priority_barriers_and_bridges_by_hand() {
    let dir = tempfile::tempdir().expect("tmp");
    let job = by_hand_fixture(dir.path());
    let out = merge(job, 256 << 20).expect("merge");
    let merged = read::<i32>(&out.join(MERGED_FILE));
    let bridges = read::<u8>(&out.join(BRIDGES_FILE));
    let n = MERGED_NODATA;
    #[rustfmt::skip]
    let expected = vec![
        1, 1, 1, 1, n, 2,
        1, 1, n, 1, n, 2,
        1, 1, n, n, 1, 2,
        1, 1, 1, 1, 1, 2,
        1003, 1003, 1003, 1, 1, 2,
    ];
    assert_eq!(merged, expected);
    let mut expected_bridges = vec![0u8; 30];
    expected_bridges[2 * 6 + 2] = 1;
    assert_eq!(bridges, expected_bridges);

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("run.json")).expect("run.json"))
            .expect("json");
    let result = &report["run"]["result"];
    assert_eq!(result["barrier_cells"], 2);
    assert_eq!(result["bridges_removed"], 1);
    assert_eq!(result["uncovered_cells"], 2);
    let road = &report["run"]["stack"][0];
    assert_eq!(road["features"]["read"], 3);
    assert_eq!(road["features"]["without_class"], 1);
    assert_eq!(road["classes"][1]["class"], 1007);
    let csv = std::fs::read_to_string(out.join(CLASSES_FILE)).expect("classes.csv");
    assert_eq!(
        csv,
        "class,label,stack_item,cells\n1,,2,17\n2,,2,5\n1003,,1,3\n"
    );
}

#[test]
fn without_cleaning_the_bridge_stays() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = by_hand_fixture(dir.path());
    job.clean_bridges = false;
    let out = merge(job, 256 << 20).expect("merge");
    assert_eq!(read::<i32>(&out.join(MERGED_FILE))[2 * 6 + 2], 1007);
    assert!(!out.join(BRIDGES_FILE).exists());
}

#[test]
fn class_conflicts_are_rejected_and_reclass_resolves_them() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = by_hand_fixture(dir.path());
    if let StackItem::Road(road) = &mut job.stack[0] {
        road.add_1000 = false;
        road.classes = Some(vec![7, 3]);
    }
    // Road class 3 and 7 do not collide with land-cover classes 1 and 2:
    // make the land cover produce 3 to collide.
    if let StackItem::LandCover(lc) = &mut job.stack[1] {
        lc.reclass = vec![Reclass { from: 2, to: 3 }];
    }
    let err = prepare(job.clone(), &settings(256 << 20, 2), &never).expect_err("conflict");
    assert_eq!(err.kind(), ErrorKind::Validation);
    let text = err.to_string();
    assert!(
        text.contains("class 3 comes from stack items 1 (road) and 2 (land_cover)"),
        "{text}"
    );

    if let StackItem::LandCover(lc) = &mut job.stack[1] {
        lc.reclass = vec![Reclass { from: 2, to: 20 }];
    }
    let out = merge(job, 256 << 20).expect("merge");
    let merged = read::<i32>(&out.join(MERGED_FILE));
    assert_eq!(merged[5], 20);
    // Class 7 is listed first: it keeps its cell, which is a bridge.
    assert_eq!(merged[4 * 6], 3);
}

#[test]
fn missing_fields_and_bad_filters_are_validation_errors() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = by_hand_fixture(dir.path());
    if let StackItem::Road(road) = &mut job.stack[0] {
        road.class_field = "kind".into();
    }
    job.stack.push(StackItem::Barrier(BarrierItem {
        path: dir.path().join("barrier.gpkg"),
        layer: None,
        filter: Some("no_such_field = 1".into()),
    }));
    let err = prepare(job, &settings(256 << 20, 2), &never).expect_err("invalid");
    let Error::Validation { issues } = err else {
        panic!("validation error expected");
    };
    assert_eq!(issues.len(), 2, "{issues:#?}");
    assert!(issues[0].contains("no field \"kind\""));
    assert!(issues[1].contains("invalid filter"));
}

#[test]
fn outputs_do_not_depend_on_memory() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path();
    let (cols, rows) = (900, 700);
    write_raster::<f32>(
        &path.join("dem.tif"),
        cols,
        rows,
        geo_transform(X0, Y0),
        vec![0.0; cols * rows],
        Some(-9999.0),
    );
    let lc: Vec<u16> = (0..cols * rows)
        .map(|i| [1, 2, 3, 0][(i / cols / 37 + i % cols / 53) % 4])
        .collect();
    write_raster::<u16>(
        &path.join("lc.tif"),
        cols,
        rows,
        geo_transform(X0, Y0),
        lc,
        Some(0.0),
    );
    let mut roads = Vec::new();
    let mut rivers = Vec::new();
    for k in 0..40 {
        let a = (k * 37 % 900) as f64 + 0.3;
        let b = (k * 53 % 700) as f64 + 0.7;
        roads.push(format!(
            "LINESTRING ({a} 0.5, {} {b}, {} 699.2)",
            (a * 1.7) % 900.0,
            (b * 0.9) % 900.0
        ));
        rivers.push(format!(
            "LINESTRING (0.2 {b}, 450.5 {}, 899.9 {})",
            (b * 1.3) % 700.0,
            a % 700.0
        ));
    }
    let road_features: Vec<(Option<i64>, &str)> = roads
        .iter()
        .enumerate()
        .map(|(i, wkt)| (Some(1 + (i % 3) as i64), wkt.as_str()))
        .collect();
    let river_features: Vec<(Option<i64>, &str)> =
        rivers.iter().map(|wkt| (None, wkt.as_str())).collect();
    write_items(&path.join("roads.gpkg"), &road_features);
    write_items(&path.join("rivers.gpkg"), &river_features);
    write_items(
        &path.join("lakes.gpkg"),
        &[(
            None,
            "POLYGON ((100.2 100.7, 600.4 150.1, 500.9 650.3, 120.5 400.2, 100.2 100.7), \
             (300 300, 350 300, 350 350, 300 350, 300 300))",
        )],
    );
    let job = |out: &str| MergeLandCoverJob {
        schema_version: SCHEMA_VERSION,
        dem: path.join("dem.tif"),
        stack: vec![
            road(path.join("roads.gpkg")),
            land_cover(path.join("lc.tif")),
            barrier(path.join("rivers.gpkg")),
            barrier(path.join("lakes.gpkg")),
        ],
        clean_bridges: true,
        output_dir: path.join(out),
    };
    // 23 MiB: the 16 MiB GDAL cache floor and buffers leave about 80 of
    // the 165 tiles resident.
    let minimum = 23 << 20;
    let tight = prepare(job("a"), &settings(minimum, 2), &never).expect("prepare at minimum");
    assert!(tight.plan.resident_tiles < tight.plan.total_tiles);
    let (out_a, report_a) = run(tight, &settings(minimum, 2), &never).expect("run a");
    assert!(
        report_a.execution.details.store.tile_writes > 0,
        "scratch not exercised"
    );
    let out_b = merge(job("b"), 4 << 30).expect("run b");
    for name in [MERGED_FILE, BRIDGES_FILE, CLASSES_FILE] {
        let bytes_a = std::fs::read(out_a.join(name)).expect("read a");
        let bytes_b = std::fs::read(out_b.join(name)).expect("read b");
        assert!(bytes_a == bytes_b, "{name} differs between budgets");
    }
    let run_part = |dir: &Path| {
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("run.json")).expect("read"))
                .expect("json");
        let mut run = report["run"].clone();
        run["job"]["output_dir"] = serde_json::Value::Null;
        run
    };
    assert_eq!(run_part(&out_a), run_part(&out_b));
    assert!(report_a.run.result.bridges_removed > 0 || report_a.run.result.bridge_cells > 0);
}

#[test]
fn layers_in_another_crs_are_reprojected_and_outside_features_counted() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = by_hand_fixture(dir.path());
    // The barrier of the fixture, in WGS 84, plus a polygon far outside.
    let ring = |cells: &[(f64, f64)]| {
        let map_points: Vec<(f64, f64)> = cells.iter().map(|&(c, r)| map(c, r)).collect();
        let points: Vec<String> = to_wgs84(&map_points)
            .iter()
            .map(|(lon, lat)| format!("{lon} {lat}"))
            .collect();
        format!("POLYGON (({}))", points.join(", "))
    };
    let barrier = ring(&[(4.1, 0.1), (4.9, 0.1), (4.9, 1.9), (4.1, 1.9), (4.1, 0.1)]);
    let far = ring(&[(40.0, 40.0), (41.0, 40.0), (41.0, 41.0), (40.0, 40.0)]);
    write_layer(
        &dir.path().join("barrier4326.gpkg"),
        "barrier",
        4326,
        ("class", OGRFieldType::OFTInteger64),
        &[(None, &barrier), (None, &far)],
    );
    job.stack[2] = StackItem::Barrier(BarrierItem {
        path: dir.path().join("barrier4326.gpkg"),
        layer: None,
        filter: None,
    });
    let prepared = prepare(job, &settings(256 << 20, 2), &never).expect("prepare");
    assert!(
        prepared
            .warnings
            .iter()
            .any(|w| w.code == "stack_item_reprojected")
    );
    let (out, report) = run(prepared, &settings(256 << 20, 2), &never).expect("run");
    let merged = read::<i32>(&out.join(MERGED_FILE));
    assert_eq!((merged[4], merged[6 + 4]), (MERGED_NODATA, MERGED_NODATA));
    assert_eq!(report.run.result.barrier_cells, 2);
    let features = report.run.stack[2].features.as_ref().expect("counts");
    assert_eq!((features.used, features.outside_grid), (1, 1));
    assert!(
        report
            .run
            .warnings
            .iter()
            .any(|w| w.code == "features_outside_grid")
    );
}

#[test]
fn non_finite_coordinates_fail_the_run() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = by_hand_fixture(dir.path());
    let (x, y) = map(0.5, 0.5);
    write_layer(
        &dir.path().join("bad.gpkg"),
        "bad",
        EPSG,
        ("class", OGRFieldType::OFTInteger64),
        &[(None, &format!("LINESTRING ({x} {y}, nan {y})"))],
    );
    job.stack.push(StackItem::Barrier(BarrierItem {
        path: dir.path().join("bad.gpkg"),
        layer: None,
        filter: None,
    }));
    let out = job.output_dir.clone();
    let err = merge(job, 256 << 20).expect_err("non-finite");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.to_string().contains("non-finite coordinates"), "{err}");
    assert!(!out.exists(), "a failed run leaves no output");
}
