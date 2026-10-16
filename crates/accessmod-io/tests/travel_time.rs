//! End-to-end travel-time runs on small GeoTIFF / GeoPackage fixtures built
//! with GDAL in a temporary directory.

use std::path::Path;

use accessmod_core::cost::{Direction, Neighbourhood};
use accessmod_core::mobility::{MobilityModel, TransportMode};
use accessmod_io::error::ErrorKind;
use accessmod_io::fixtures::{
    EPSG, X0, Y0, geo_transform, grid, never, read, run_job, settings, to_wgs84, write_layer,
    write_raster,
};
use accessmod_io::job::{
    FacilityIssues, FacilitySource, MethodName, OutputOptions, SCHEMA_VERSION, ScenarioEntry,
    TravelTimeJob,
};
use accessmod_io::output::{FLOAT_NODATA, NEAREST_FILE, TRAVEL_TIME_FILE};
use accessmod_io::travel_time::{TravelTime, prepare, run};
use gdal::vector::{FieldValue, OGRFieldType};

/// Facility layer `facilities` with points `(hf_id, x, y)` in EPSG `epsg`.
fn write_points(path: &Path, epsg: u32, points: &[(i64, f64, f64)]) {
    let wkts: Vec<String> = points
        .iter()
        .map(|(_, x, y)| format!("POINT ({x} {y})"))
        .collect();
    let features: Vec<(Option<FieldValue>, &str)> = points
        .iter()
        .zip(&wkts)
        .map(|((id, _, _), wkt)| (Some(FieldValue::Integer64Value(*id)), wkt.as_str()))
        .collect();
    write_layer(
        path,
        "facilities",
        epsg,
        ("hf_id", OGRFieldType::OFTInteger64),
        &features,
    );
}

/// A `cols × rows` flat landscape of class 1, with facilities at the given
/// cells, plus a job running on it.
struct Fixture {
    dir: tempfile::TempDir,
    job: TravelTimeJob,
}

impl Fixture {
    fn new(cols: usize, rows: usize, facility_cells: &[(usize, usize)]) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path();
        write_raster::<f32>(
            &path.join("dem.tif"),
            cols,
            rows,
            geo_transform(X0, Y0),
            vec![0.0; cols * rows],
            Some(-9999.0),
        );
        write_raster::<u8>(
            &path.join("lc.tif"),
            cols,
            rows,
            geo_transform(X0, Y0),
            vec![1; cols * rows],
            Some(0.0),
        );
        let points: Vec<(i64, f64, f64)> = facility_cells
            .iter()
            .enumerate()
            .map(|(i, &(r, c))| {
                let (x, y) = grid(cols, rows).cell_centre(r, c);
                (100 + i as i64, x, y)
            })
            .collect();
        write_points(&path.join("hf.gpkg"), EPSG, &points);
        let job = TravelTimeJob {
            schema_version: SCHEMA_VERSION,
            dem: path.join("dem.tif"),
            land_cover: path.join("lc.tif"),
            facilities: FacilitySource {
                path: path.join("hf.gpkg"),
                layer: None,
                id_field: "hf_id".into(),
                filter: None,
            },
            scenario: vec![ScenarioEntry {
                class: 1,
                mode: TransportMode::Motorized,
                speed_kmh: 5.0,
                label: None,
            }],
            method: MethodName::Isotropic,
            direction: Direction::FromFacilities,
            neighbourhood: Neighbourhood::Eight,
            limit_s: None,
            mobility_model: MobilityModel::Accessmod5V1,
            facility_issues: FacilityIssues::Error,
            outputs: OutputOptions { direction: true },
            output_dir: path.join("out"),
        };
        Self { dir, job }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

#[test]
fn corridor_by_hand() {
    let fixture = Fixture::new(5, 3, &[(1, 0)]);
    let out = run_job::<TravelTime>(fixture.job.clone(), &settings(256 << 20, 1)).expect("run");
    let time: Vec<f64> = read(&out.join(TRAVEL_TIME_FILE));
    // Middle row: 72 s per 100 m step at 5 km/h.
    for c in 0..5 {
        assert!(
            (time[5 + c] - 72.0 * c as f64).abs() < 1e-9,
            "col {c}: {}",
            time[5 + c]
        );
    }
    // Diagonal neighbour: 141.42 m at 5 km/h.
    assert!((time[1] - 2f64.sqrt() * 72.0).abs() < 1e-9);
    let nearest: Vec<u32> = read(&out.join(NEAREST_FILE));
    assert!(nearest.iter().all(|&n| n == 1));
    let distance: Vec<f64> = read(&out.join("path_distance_m.tif"));
    assert_eq!(distance[5 + 4], 400.0);
    let direction: Vec<u8> = read(&out.join("direction.tif"));
    assert_eq!(direction[5], 0, "facility cell");
    assert_eq!(direction[6], 2, "reached eastwards");

    for name in ["run.json", "job.resolved.json", "nearest_facility.json"] {
        assert!(out.join(name).is_file(), "{name}");
    }
    let table: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("nearest_facility.json")).expect("read"),
    )
    .expect("json");
    assert_eq!(table["facilities"][0]["id"], 100);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("run.json")).expect("read"))
            .expect("json");
    assert_eq!(report["run"]["result"]["reached_cells"], 15);
    assert!(
        report["run"]["deviations"]
            .as_array()
            .is_some_and(|d| !d.is_empty())
    );
    let leftovers: Vec<_> = std::fs::read_dir(fixture.path())
        .expect("list")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("accessmod-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "staging directory left behind");
}

#[test]
fn barriers_and_limit_leave_nodata() {
    let mut fixture = Fixture::new(5, 1, &[(0, 0)]);
    fixture.job.limit_s = Some(144.0);
    let out = run_job::<TravelTime>(fixture.job.clone(), &settings(256 << 20, 1)).expect("run");
    let time: Vec<f64> = read(&out.join(TRAVEL_TIME_FILE));
    assert_eq!(time, vec![0.0, 72.0, 144.0, FLOAT_NODATA, FLOAT_NODATA]);
}

#[test]
fn outputs_do_not_depend_on_memory_or_threads() {
    let fixture = Fixture::new(150, 130, &[(10, 10), (120, 140), (60, 75)]);
    let mut a = fixture.job.clone();
    a.output_dir = fixture.path().join("a");
    a.neighbourhood = Neighbourhood::Sixteen;
    let mut b = a.clone();
    b.output_dir = fixture.path().join("b");
    let out_a = run_job::<TravelTime>(a, &settings(256 << 20, 1)).expect("run a");
    let out_b = run_job::<TravelTime>(b, &settings(4 << 30, 4)).expect("run b");
    for name in [
        TRAVEL_TIME_FILE,
        NEAREST_FILE,
        "path_distance_m.tif",
        "direction.tif",
        "nearest_facility.json",
    ] {
        let bytes_a = std::fs::read(out_a.join(name)).expect("read a");
        let bytes_b = std::fs::read(out_b.join(name)).expect("read b");
        assert!(bytes_a == bytes_b, "{name} differs between runs");
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
}

#[test]
fn misaligned_land_cover_is_rejected() {
    let fixture = Fixture::new(5, 3, &[(1, 0)]);
    write_raster::<u8>(
        &fixture.path().join("lc.tif"),
        5,
        3,
        geo_transform(X0 + 50.0, Y0),
        vec![1; 15],
        Some(0.0),
    );
    let err =
        prepare(fixture.job.clone(), &settings(256 << 20, 1), &never).expect_err("misaligned");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.to_string().contains("lattice"), "{err}");
}

#[test]
fn larger_land_cover_on_the_lattice_is_windowed() {
    let fixture = Fixture::new(5, 3, &[(1, 0)]);
    // 9 x 7 cells starting 2 columns west and 2 rows north of the DEM.
    write_raster::<u8>(
        &fixture.path().join("lc.tif"),
        9,
        7,
        geo_transform(X0 - 200.0, Y0 + 200.0),
        vec![1; 63],
        Some(0.0),
    );
    let out = run_job::<TravelTime>(fixture.job.clone(), &settings(256 << 20, 1)).expect("run");
    let time: Vec<f64> = read(&out.join(TRAVEL_TIME_FILE));
    assert_eq!(time.len(), 15);
    assert!((time[5 + 4] - 288.0).abs() < 1e-9);
}

#[test]
fn facility_problems_fail_or_warn() {
    // Second facility is outside the grid (row 5 of a 3-row grid).
    let fixture = Fixture::new(5, 3, &[(1, 0), (5, 1)]);
    let err = prepare(fixture.job.clone(), &settings(256 << 20, 1), &never).expect_err("outside");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.to_string().contains("outside the DEM grid"), "{err}");

    let mut job = fixture.job.clone();
    job.facility_issues = FacilityIssues::Skip;
    let prepared = prepare(job, &settings(256 << 20, 1), &never).expect("skip mode");
    assert_eq!(prepared.facilities.kept.len(), 1);
    assert!(
        prepared
            .warnings
            .iter()
            .any(|w| w.code == "facility_skipped")
    );
}

#[test]
fn facilities_in_another_crs_are_reprojected() {
    let fixture = Fixture::new(5, 3, &[]);
    // Centre of cell (1, 2) in EPSG:32632, expressed in WGS 84 lon/lat.
    let [(lon, lat)] = to_wgs84(&[(X0 + 250.0, Y0 - 150.0)])[..] else {
        unreachable!("one point in, one out")
    };
    std::fs::remove_file(fixture.path().join("hf.gpkg")).expect("remove");
    write_points(&fixture.path().join("hf.gpkg"), 4326, &[(7, lon, lat)]);
    let prepared = prepare(fixture.job.clone(), &settings(256 << 20, 1), &never).expect("prepare");
    assert!(prepared.facilities.reprojected);
    let f = &prepared.facilities.kept[0];
    assert_eq!((f.row, f.col), (1, 2));
    assert!(
        prepared
            .warnings
            .iter()
            .any(|w| w.code == "facilities_reprojected")
    );
}

#[test]
fn non_empty_output_dir_is_rejected() {
    let fixture = Fixture::new(5, 3, &[(1, 0)]);
    std::fs::create_dir(&fixture.job.output_dir).expect("mkdir");
    std::fs::write(fixture.job.output_dir.join("keep.txt"), "x").expect("write");
    let err = prepare(fixture.job.clone(), &settings(256 << 20, 1), &never).expect_err("non-empty");
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn cancellation_leaves_nothing_behind() {
    let fixture = Fixture::new(5, 3, &[(1, 0)]);
    let settings = settings(256 << 20, 1);
    let prepared = prepare(fixture.job.clone(), &settings, &never).expect("prepare");
    let err = run(prepared, &settings, &|| true).expect_err("cancelled");
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(!fixture.job.output_dir.exists());
    let leftovers = std::fs::read_dir(fixture.path())
        .expect("list")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("accessmod-tmp"))
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn wide_grids_cross_read_and_write_chunks() {
    // Wider than one read chunk (4096 columns) and one output block (256).
    let cols = accessmod_io::raster::CHUNK_COLS + 4;
    let fixture = Fixture::new(cols, 2, &[(0, 0)]);
    // Land cover on the lattice, shifted 3 columns east: the first 3 DEM
    // columns are uncovered (barriers) except that the facility needs one.
    write_raster::<u8>(
        &fixture.path().join("lc.tif"),
        cols,
        2,
        geo_transform(X0 + 300.0, Y0),
        vec![1; cols * 2],
        Some(0.0),
    );
    let mut job = fixture.job.clone();
    job.facility_issues = FacilityIssues::Skip;
    let err =
        prepare(job.clone(), &settings(256 << 20, 1), &never).expect_err("facility on barrier");
    assert_eq!(err.kind(), ErrorKind::Validation, "{err}");

    // Move the facility onto covered land.
    std::fs::remove_file(fixture.path().join("hf.gpkg")).expect("remove");
    write_points(
        &fixture.path().join("hf.gpkg"),
        EPSG,
        &[(1, X0 + 350.0, Y0 - 50.0)],
    );
    let out = run_job::<TravelTime>(job, &settings(256 << 20, 2)).expect("run");
    let time: Vec<f64> = read(&out.join(TRAVEL_TIME_FILE));
    assert_eq!(time[2], FLOAT_NODATA, "uncovered column is a barrier");
    assert_eq!(time[3], 0.0);
    let last = cols - 1;
    assert!(
        (time[last] - 72.0 * (last - 3) as f64).abs() < 1e-6,
        "{}",
        time[last]
    );
    let second_row_last = cols + last;
    assert!(time[second_row_last] > time[last]);
}
