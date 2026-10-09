//! End-to-end travel-time runs on small GeoTIFF / GeoPackage fixtures built
//! with GDAL in a temporary directory.

use std::path::{Path, PathBuf};

use accessmod_core::cost::{Direction, Neighbourhood};
use accessmod_core::mobility::{MobilityModel, TransportMode};
use accessmod_io::error::{Error, ErrorKind};
use accessmod_io::job::{
    FacilityIssues, FacilitySource, MethodName, OutputOptions, SCHEMA_VERSION, ScenarioEntry,
    TravelTimeJob,
};
use accessmod_io::output::{FLOAT_NODATA, NEAREST_FILE, TRAVEL_TIME_FILE};
use accessmod_io::travel_time::{ExecutionSettings, prepare, run};
use gdal::raster::{Buffer, GdalType};
use gdal::spatial_ref::SpatialRef;
use gdal::vector::{
    Feature, FieldValue, Geometry, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType,
};
use gdal::{Dataset, DriverManager};

const EPSG: u32 = 32632;
const X0: f64 = 500_000.0;
const Y0: f64 = 5_000_000.0;
const RES: f64 = 100.0;

fn geo_transform(x0: f64, y0: f64) -> [f64; 6] {
    [x0, RES, 0.0, y0, 0.0, -RES]
}

fn write_raster<T: GdalType + Copy>(
    path: &Path,
    cols: usize,
    rows: usize,
    gt: [f64; 6],
    data: Vec<T>,
    nodata: Option<f64>,
) {
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver");
    let mut ds = driver
        .create_with_band_type::<T, _>(path, cols, rows, 1)
        .expect("create raster");
    ds.set_geo_transform(&gt).expect("geotransform");
    ds.set_spatial_ref(&SpatialRef::from_epsg(EPSG).expect("srs"))
        .expect("srs");
    let mut band = ds.rasterband(1).expect("band");
    band.set_no_data_value(nodata).expect("nodata");
    let mut buffer = Buffer::new((cols, rows), data);
    band.write((0, 0), (cols, rows), &mut buffer)
        .expect("write");
}

fn write_points(path: &Path, epsg: u32, points: &[(i64, f64, f64)]) {
    let driver = DriverManager::get_driver_by_name("GPKG").expect("GPKG driver");
    let mut ds = driver.create_vector_only(path).expect("create gpkg");
    let srs = SpatialRef::from_epsg(epsg).expect("srs");
    let layer = ds
        .create_layer(LayerOptions {
            name: "facilities",
            srs: Some(&srs),
            ty: OGRwkbGeometryType::wkbPoint,
            options: None,
        })
        .expect("layer");
    layer
        .create_defn_fields(&[("hf_id", OGRFieldType::OFTInteger64)])
        .expect("fields");
    let field = layer.defn().field_index("hf_id").expect("field");
    for &(id, x, y) in points {
        let geometry = Geometry::from_wkt(&format!("POINT ({x} {y})")).expect("point");
        let mut feature = Feature::new(layer.defn()).expect("feature");
        feature.set_geometry(geometry).expect("geometry");
        feature
            .set_field(field, &FieldValue::Integer64Value(id))
            .expect("id");
        feature.create(&layer).expect("create feature");
    }
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
                (
                    100 + i as i64,
                    X0 + (c as f64 + 0.5) * RES,
                    Y0 - (r as f64 + 0.5) * RES,
                )
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

fn settings(memory_bytes: u64, threads: usize) -> ExecutionSettings {
    ExecutionSettings {
        memory_bytes,
        threads,
        tmp_dir: std::env::temp_dir(),
    }
}

fn never() -> bool {
    false
}

fn run_job(job: TravelTimeJob, settings: &ExecutionSettings) -> Result<PathBuf, Error> {
    let prepared = prepare(job, settings, &never)?;
    run(prepared, settings, &never).map(|(dir, _)| dir)
}

fn read<T: GdalType + Copy>(path: &Path) -> Vec<T> {
    let ds = Dataset::open(path).expect("open output");
    let band = ds.rasterband(1).expect("band");
    let (cols, rows) = ds.raster_size();
    band.read_as::<T>((0, 0), (cols, rows), (cols, rows), None)
        .expect("read")
        .data()
        .to_vec()
}

#[test]
fn corridor_by_hand() {
    let fixture = Fixture::new(5, 3, &[(1, 0)]);
    let out = run_job(fixture.job.clone(), &settings(256 << 20, 1)).expect("run");
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
    let out = run_job(fixture.job.clone(), &settings(256 << 20, 1)).expect("run");
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
    let out_a = run_job(a, &settings(256 << 20, 1)).expect("run a");
    let out_b = run_job(b, &settings(4 << 30, 4)).expect("run b");
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
    let out = run_job(fixture.job.clone(), &settings(256 << 20, 1)).expect("run");
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
    let (x, y) = (X0 + 250.0, Y0 - 150.0);
    let utm = SpatialRef::from_epsg(EPSG).expect("srs");
    let mut wgs84 = SpatialRef::from_epsg(4326).expect("srs");
    wgs84.set_axis_mapping_strategy(gdal::spatial_ref::AxisMappingStrategy::TraditionalGisOrder);
    let mut utm_traditional = utm.clone();
    utm_traditional
        .set_axis_mapping_strategy(gdal::spatial_ref::AxisMappingStrategy::TraditionalGisOrder);
    let transform =
        gdal::spatial_ref::CoordTransform::new(&utm_traditional, &wgs84).expect("transform");
    let (mut xs, mut ys) = ([x], [y]);
    transform
        .transform_coords(&mut xs, &mut ys, &mut [])
        .expect("to wgs84");
    std::fs::remove_file(fixture.path().join("hf.gpkg")).expect("remove");
    write_points(&fixture.path().join("hf.gpkg"), 4326, &[(7, xs[0], ys[0])]);
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
