//! Optimization on small fixtures built with GDAL.

use std::path::Path;

use accessmod_io::fixtures::{
    EPSG, X0, Y0, cell_polygon, csv_table, geo_transform, grid, run_job, settings, write_features,
    write_raster,
};
use accessmod_io::job::{FacilitySource, SCHEMA_VERSION};
use accessmod_io::optimization::{OPTIMIZATION_FILE, Optimization, OptimizationJob, ZoneQuotaJob};
use accessmod_io::zonal::ZoneLayer;
use gdal::vector::{FieldValue, OGRFieldType};

/// One person per cell on `cols × rows` cells; catchments `(id, WKT)`.
fn fixture(dir: &Path, cols: usize, rows: usize, catchments: &[(i64, String)]) -> OptimizationJob {
    let gt = geo_transform(X0, Y0);
    write_raster::<f32>(
        &dir.join("pop.tif"),
        cols,
        rows,
        gt,
        vec![1.0; cols * rows],
        Some(-1.0),
    );
    let features: Vec<(&str, Vec<Option<FieldValue>>)> = catchments
        .iter()
        .map(|(id, wkt)| (wkt.as_str(), vec![Some(FieldValue::Integer64Value(*id))]))
        .collect();
    write_features(
        &dir.join("catchments.gpkg"),
        "catchments",
        EPSG,
        &[("id", OGRFieldType::OFTInteger64)],
        &features,
    );
    OptimizationJob {
        schema_version: SCHEMA_VERSION,
        population: dir.join("pop.tif"),
        catchments: FacilitySource {
            path: dir.join("catchments.gpkg"),
            layer: None,
            id_field: "id".into(),
            filter: None,
            ids: None,
        },
        max_facilities: 5,
        zone_quota: None,
        output_dir: dir.join("out"),
    }
}

fn column(out: &Path, name: &str) -> Vec<String> {
    let table = csv_table(&out.join(OPTIMIZATION_FILE), "rank");
    (1..=table.len())
        .map(|rank| table[&rank.to_string()][name].clone())
        .collect()
}

fn numbers(out: &Path, name: &str) -> Vec<f64> {
    column(out, name)
        .iter()
        .map(|v| v.parse().expect("number"))
        .collect()
}

#[test]
fn overlaps_contained_duplicates_and_fractions() {
    let dir = tempfile::tempdir().expect("tmp");
    let job = fixture(
        dir.path(),
        6,
        2,
        &[
            // 1: cells 0..3 (6 people); 9: the same geometry.
            (1, cell_polygon(0.0, 0.0, 3.0, 2.0)),
            // 2: cells 2..5, one column shared with 1.
            (2, cell_polygon(2.0, 0.0, 5.0, 2.0)),
            // 3: inside 2.
            (3, cell_polygon(3.0, 0.0, 4.0, 1.0)),
            (9, cell_polygon(0.0, 0.0, 3.0, 2.0)),
            // 5: half a cell.
            (5, cell_polygon(5.0, 0.0, 5.5, 1.0)),
        ],
    );
    let out = run_job::<Optimization>(job.clone(), &settings(64 << 20, 1)).expect("run");
    // 1 and 2 tie (6, 6): the first wins; 2 then adds 4 people, 5 half a
    // person; 3, inside 2, is appended last with nothing added.
    assert_eq!(column(&out, "facility_id"), ["1", "2", "5", "3"]);
    assert_eq!(column(&out, "facility_ids"), ["1 // 9", "2", "5", "3"]);
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
    assert!(close(
        &numbers(&out, "population_covered"),
        &[6.0, 4.0, 0.5, 0.0]
    ));
    assert!(close(
        &numbers(&out, "population_catchment"),
        &[6.0, 6.0, 0.5, 1.0]
    ));
    assert!(close(
        &numbers(&out, "population_covered_cumulative"),
        &[6.0, 10.0, 10.5, 10.5]
    ));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("run.json")).expect("run.json"))
            .expect("json");
    let result = &report["run"]["result"];
    assert_eq!(result["stop_reason"], "no_catchment_left");
    assert_eq!(result["duplicate_catchments"], 1);

    // Same table whatever the budget and threads.
    let mut other = job;
    other.output_dir = dir.path().join("other");
    let again = run_job::<Optimization>(other, &settings(4 << 30, 4)).expect("run");
    assert_eq!(
        std::fs::read(out.join(OPTIMIZATION_FILE)).expect("a"),
        std::fs::read(again.join(OPTIMIZATION_FILE)).expect("b")
    );
}

#[test]
fn zone_minimum_takes_a_facility_in_every_zone() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = fixture(
        dir.path(),
        8,
        2,
        &[
            (1, cell_polygon(0.0, 0.0, 3.0, 2.0)),
            (2, cell_polygon(3.0, 0.0, 5.0, 2.0)),
            (3, cell_polygon(6.0, 0.0, 7.0, 2.0)),
        ],
    );
    job.max_facilities = 2;
    let free = run_job::<Optimization>(job.clone(), &settings(64 << 20, 1)).expect("run");
    assert_eq!(column(&free, "facility_id"), ["1", "2"]);

    let zone = |id: &str, c0: f64, c1: f64| {
        (
            cell_polygon(c0, 0.0, c1, 2.0),
            vec![Some(FieldValue::StringValue(id.into()))],
        )
    };
    let zones = [zone("west", 0.0, 5.5), zone("east", 5.5, 8.0)];
    let zones: Vec<(&str, Vec<Option<FieldValue>>)> =
        zones.iter().map(|(w, v)| (w.as_str(), v.clone())).collect();
    write_features(
        &dir.path().join("zones.gpkg"),
        "zones",
        EPSG,
        &[("name", OGRFieldType::OFTString)],
        &zones,
    );
    let g = grid(8, 2);
    let points: Vec<(String, Vec<Option<FieldValue>>)> = [(1, 1), (2, 4), (3, 6)]
        .iter()
        .map(|&(id, col)| {
            let (x, y) = g.cell_centre(0, col);
            (
                format!("POINT ({x} {y})"),
                vec![Some(FieldValue::Integer64Value(id))],
            )
        })
        .collect();
    let points: Vec<(&str, Vec<Option<FieldValue>>)> = points
        .iter()
        .map(|(w, v)| (w.as_str(), v.clone()))
        .collect();
    write_features(
        &dir.path().join("facilities.gpkg"),
        "facilities",
        EPSG,
        &[("code", OGRFieldType::OFTInteger64)],
        &points,
    );
    job.output_dir = dir.path().join("quota");
    job.zone_quota = Some(ZoneQuotaJob {
        zones: ZoneLayer {
            path: dir.path().join("zones.gpkg"),
            layer: None,
            id_field: "name".into(),
            label_field: None,
            filter: None,
        },
        facilities: FacilitySource {
            path: dir.path().join("facilities.gpkg"),
            layer: None,
            id_field: "code".into(),
            filter: None,
            ids: None,
        },
        min_per_zone: 1,
    });
    let out = run_job::<Optimization>(job.clone(), &settings(64 << 20, 1)).expect("run");
    assert_eq!(column(&out, "facility_id"), ["1", "3"]);
    assert_eq!(column(&out, "zone_id"), ["west", "east"]);

    // Two zones of one facility each exceed one facility.
    job.max_facilities = 1;
    job.output_dir = dir.path().join("too-few");
    let error = run_job::<Optimization>(job, &settings(64 << 20, 1)).expect_err("quota");
    assert!(
        error.to_string().contains("exceeds max_facilities"),
        "{error}"
    );
}

#[test]
fn zero_population_candidates_drop_only_after_an_overlap() {
    let dir = tempfile::tempdir().expect("tmp");
    let catchments = [
        // 1 holds a NoData cell and an empty one: 1 person.
        (1, cell_polygon(0.0, 0.0, 3.0, 1.0)),
        (2, cell_polygon(4.0, 0.0, 5.0, 1.0)),
        // 3: nobody, far from the others.
        (3, cell_polygon(7.0, 0.0, 8.0, 1.0)),
    ];
    let mut job = fixture(dir.path(), 8, 1, &catchments);
    let population = vec![1.0, -1.0, 0.0, 5.0, 2.0, 0.0, 0.0, 0.0];
    write_raster::<f32>(
        &dir.path().join("pop.tif"),
        8,
        1,
        geo_transform(X0, Y0),
        population,
        Some(-1.0),
    );
    // No selection reduces a catchment: 3 stays, last, covering no one.
    let out = run_job::<Optimization>(job.clone(), &settings(64 << 20, 1)).expect("run");
    assert_eq!(column(&out, "facility_id"), ["2", "1", "3"]);
    assert_eq!(numbers(&out, "population_covered"), [2.0, 1.0, 0.0]);

    // 4 touches 2 (and is reduced by it): candidates covering no one go.
    let mut with_neighbour = catchments.to_vec();
    with_neighbour.push((4, cell_polygon(5.0, 0.0, 6.0, 1.0)));
    let features: Vec<(&str, Vec<Option<FieldValue>>)> = with_neighbour
        .iter()
        .map(|(id, wkt)| (wkt.as_str(), vec![Some(FieldValue::Integer64Value(*id))]))
        .collect();
    write_features(
        &dir.path().join("neighbour.gpkg"),
        "catchments",
        EPSG,
        &[("id", OGRFieldType::OFTInteger64)],
        &features,
    );
    job.catchments.path = dir.path().join("neighbour.gpkg");
    job.output_dir = dir.path().join("neighbour");
    let out = run_job::<Optimization>(job, &settings(64 << 20, 1)).expect("run");
    assert_eq!(column(&out, "facility_id"), ["2", "1"]);
}
