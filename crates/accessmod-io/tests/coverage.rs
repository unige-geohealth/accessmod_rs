//! Geographic coverage on small fixtures built with GDAL.

use std::path::Path;

use accessmod_core::cost::{Direction, Neighbourhood};
use accessmod_core::mobility::TransportMode;
use accessmod_io::coverage::{
    BARRIERS_FILE, CAPACITY_FILE, CATCHMENTS_FILE, CoverageJob, CoverageOrder, GeographicCoverage,
    RESIDUAL_FILE, ZONES_FILE, prepare, run,
};
use accessmod_io::fixtures::{
    EPSG, X0, Y0, cell_polygon, geo_transform, grid, never, read, read_valid, run_job, settings,
    write_features, write_raster,
};
use accessmod_io::job::{
    FacilityIssues, FacilitySource, MethodName, SCHEMA_VERSION, ScenarioEntry,
};
use accessmod_io::zonal::ZoneLayer;
use gdal::vector::{FieldValue, OGRFieldType};

/// Facilities `(id, capacity, row, col)` at cell centres of a `cols × rows`
/// fixture grid.
fn write_facilities(
    path: &Path,
    cols: usize,
    rows: usize,
    facilities: &[(i64, f64, usize, usize)],
) {
    let g = grid(cols, rows);
    let wkts: Vec<String> = facilities
        .iter()
        .map(|&(_, _, r, c)| {
            let (x, y) = g.cell_centre(r, c);
            format!("POINT ({x} {y})")
        })
        .collect();
    let features: Vec<(&str, Vec<Option<FieldValue>>)> = facilities
        .iter()
        .zip(&wkts)
        .map(|(&(id, capacity, _, _), wkt)| {
            (
                wkt.as_str(),
                vec![
                    Some(FieldValue::Integer64Value(id)),
                    Some(FieldValue::RealValue(capacity)),
                ],
            )
        })
        .collect();
    write_features(
        path,
        "facilities",
        EPSG,
        &[
            ("id", OGRFieldType::OFTInteger64),
            ("capacity", OGRFieldType::OFTReal),
        ],
        &features,
    );
}

/// Flat land of class 1 (walking 6 km/h: 60 s per 100 m cell), 10 people
/// per cell.
fn fixture(dir: &Path, cols: usize, rows: usize, limit_s: f64) -> CoverageJob {
    let gt = geo_transform(X0, Y0);
    let n = cols * rows;
    write_raster::<f32>(&dir.join("dem.tif"), cols, rows, gt, vec![0.0; n], None);
    write_raster::<u8>(&dir.join("lc.tif"), cols, rows, gt, vec![1; n], Some(0.0));
    write_raster::<f32>(&dir.join("pop.tif"), cols, rows, gt, vec![10.0; n], None);
    CoverageJob {
        schema_version: SCHEMA_VERSION,
        dem: dir.join("dem.tif"),
        land_cover: dir.join("lc.tif"),
        population: dir.join("pop.tif"),
        facilities: FacilitySource {
            path: dir.join("hf.gpkg"),
            layer: None,
            id_field: "id".into(),
            filter: None,
        },
        capacity_field: Some("capacity".into()),
        name_field: None,
        order: CoverageOrder::Field {
            field: "capacity".into(),
            descending: false,
        },
        scenario: vec![ScenarioEntry {
            class: 1,
            mode: TransportMode::Walking,
            speed_kmh: 6.0,
            label: None,
        }],
        method: MethodName::Isotropic,
        direction: Direction::FromFacilities,
        neighbourhood: Neighbourhood::Eight,
        limit_s,
        band_s: 60.0,
        mobility_model: Default::default(),
        ignore_capacity: false,
        remove_covered: true,
        catchments: true,
        population_on_barriers: false,
        zones: None,
        facility_issues: FacilityIssues::Error,
        output_dir: dir.join("out"),
    }
}

#[test]
fn corridor_by_hand() {
    let dir = tempfile::tempdir().expect("tmp");
    let job = fixture(dir.path(), 5, 1, 300.0);
    // Facility 1 (capacity 25) at the west end, facility 2 (100) at the east
    // end; ascending capacity: 1 first.
    write_facilities(
        &job.facilities.path,
        5,
        1,
        &[(1, 25.0, 0, 0), (2, 100.0, 0, 4)],
    );
    let out = run_job::<GeographicCoverage>(job, &settings(256 << 20, 1)).expect("run");
    let table = std::fs::read_to_string(out.join(CAPACITY_FILE)).expect("table");
    let rows: Vec<Vec<&str>> = table
        .lines()
        .skip(1)
        .map(|l| l.split(',').collect())
        .collect();
    // Facility 1: bands 0..4 hold 10 each; 25 = bands 0–1 and half of band 2
    // (case A, catchment up to 120 s).
    assert_eq!(rows[0][1], "1");
    assert_eq!((rows[0][8], rows[0][10], rows[0][16]), ("120", "25", "A"));
    assert_eq!((rows[0][13], rows[0][14]), ("50", "25"));
    assert_eq!(rows[0][15], "50");
    // Facility 2 then reaches the 25 people left: case C, 75 unused.
    assert_eq!((rows[1][10], rows[1][11], rows[1][16]), ("25", "75", "C"));
    assert_eq!(rows[1][15], "100");
    assert_eq!(read::<f64>(&out.join(RESIDUAL_FILE)), vec![0.0; 5]);
    assert!(out.join(CATCHMENTS_FILE).exists());
}

#[test]
fn outputs_do_not_depend_on_memory() {
    let dir = tempfile::tempdir().expect("tmp");
    let (cols, rows) = (900, 700);
    let mut job = fixture(dir.path(), cols, rows, 3000.0);
    // A barrier strip (class 9) holding people.
    let land_cover = (0..cols * rows)
        .map(|i| {
            if i % cols == 600 && i / cols < 500 {
                9
            } else {
                1
            }
        })
        .collect();
    write_raster::<u8>(
        &dir.path().join("lc.tif"),
        cols,
        rows,
        geo_transform(X0, Y0),
        land_cover,
        Some(0.0),
    );
    let zones = dir.path().join("zones.gpkg");
    let (west, centre) = (
        cell_polygon(0.0, 0.0, 450.0, 700.0),
        cell_polygon(300.0, 200.0, 700.0, 500.0),
    );
    write_features(
        &zones,
        "zones",
        EPSG,
        &[("zone", OGRFieldType::OFTInteger64)],
        &[
            (west.as_str(), vec![Some(FieldValue::Integer64Value(1))]),
            (centre.as_str(), vec![Some(FieldValue::Integer64Value(2))]),
        ],
    );
    job.population_on_barriers = true;
    job.zones = Some(ZoneLayer {
        path: zones,
        layer: None,
        id_field: "zone".into(),
        label_field: None,
        filter: None,
    });
    write_facilities(
        &job.facilities.path,
        cols,
        rows,
        &[
            (1, 5000.0, 100, 100),
            (2, 80000.0, 350, 450),
            (3, 2000.0, 600, 800),
            (4, 9000.0, 360, 470),
        ],
    );
    let with_dir = |out: &str| {
        let mut job = job.clone();
        job.output_dir = dir.path().join(out);
        job
    };
    let tight_budget = 40 << 20;
    let tight = prepare(with_dir("a"), &settings(tight_budget, 1), &never).expect("prepare");
    assert!(tight.plan.resident_tiles < tight.plan.total_tiles);
    let (out_a, report_a) = run(tight, &settings(tight_budget, 1), &never).expect("run a");
    assert!(
        report_a.execution.details.residual_store.tile_writes > 0,
        "scratch not exercised"
    );
    let out_b = run_job::<GeographicCoverage>(with_dir("b"), &settings(4 << 30, 4)).expect("run b");
    for name in [
        CAPACITY_FILE,
        RESIDUAL_FILE,
        CATCHMENTS_FILE,
        BARRIERS_FILE,
        ZONES_FILE,
    ] {
        let a = std::fs::read(out_a.join(name)).expect("read a");
        let b = std::fs::read(out_b.join(name)).expect("read b");
        assert!(a == b, "{name} differs between budgets");
    }
}

#[test]
fn barriers_zones_and_population_order() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut job = fixture(dir.path(), 4, 1, 300.0);
    // The east cell is a barrier (class 9 is not in the scenario).
    write_raster::<u8>(
        &dir.path().join("lc.tif"),
        4,
        1,
        geo_transform(X0, Y0),
        vec![1, 1, 1, 9],
        Some(0.0),
    );
    write_facilities(&job.facilities.path, 4, 1, &[(1, 15.0, 0, 0)]);
    let zones = dir.path().join("zones.gpkg");
    let (west, east) = (
        cell_polygon(0.0, 0.0, 2.0, 1.0),
        cell_polygon(2.0, 0.0, 4.0, 1.0),
    );
    write_features(
        &zones,
        "zones",
        EPSG,
        &[
            ("zone", OGRFieldType::OFTInteger64),
            ("name", OGRFieldType::OFTString),
        ],
        &[
            (
                east.as_str(),
                vec![
                    Some(FieldValue::Integer64Value(2)),
                    Some(FieldValue::StringValue("east".into())),
                ],
            ),
            (
                west.as_str(),
                vec![
                    Some(FieldValue::Integer64Value(1)),
                    Some(FieldValue::StringValue("west".into())),
                ],
            ),
        ],
    );
    job.order = CoverageOrder::Population {
        limit_s: 300.0,
        descending: true,
    };
    job.population_on_barriers = true;
    job.zones = Some(ZoneLayer {
        path: zones,
        layer: None,
        id_field: "zone".into(),
        label_field: Some("name".into()),
        filter: None,
    });
    let out = run_job::<GeographicCoverage>(job, &settings(256 << 20, 1)).expect("run");
    // Bands 0–2 hold 10 each; capacity 15: band 0 and half of band 1.
    let table = std::fs::read_to_string(out.join(CAPACITY_FILE)).expect("table");
    let row: Vec<&str> = table.lines().nth(1).expect("row").split(',').collect();
    assert_eq!((row[4], row[16]), ("30", "A"));
    assert_eq!((row[13], row[14]), ("30", "15"));
    assert_eq!(
        read_valid(&out.join(BARRIERS_FILE)),
        vec![None, None, None, Some(10.0)]
    );
    assert_eq!(
        read_valid(&out.join(RESIDUAL_FILE)),
        vec![Some(0.0), Some(5.0), Some(10.0), None]
    );
    let zonal = std::fs::read_to_string(out.join(ZONES_FILE)).expect("zonal");
    let lines: Vec<&str> = zonal.lines().collect();
    assert_eq!(
        lines[1..],
        ["1,west,20,0,15,75", "2,east,20,10,0,0"],
        "{zonal}"
    );
    let run: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("run.json")).expect("run.json"))
            .expect("json");
    let result = &run["run"]["result"];
    assert_eq!(
        (
            &result["population_total"],
            &result["population_on_barriers"],
            &result["population"]
        ),
        (&40.0.into(), &10.0.into(), &30.0.into())
    );
}
