//! Scaling up on small fixtures built with GDAL.

use std::path::Path;

use accessmod_core::cost::{Direction, Neighbourhood};
use accessmod_core::mobility::TransportMode;
use accessmod_io::analysis::Monitor;
use accessmod_io::fixtures::{
    EPSG, X0, Y0, cell_polygon, geo_transform, grid, read, run_job, settings, write_features,
    write_raster,
};
use accessmod_io::job::{FacilityIssues, MethodName, SCHEMA_VERSION, ScenarioEntry};
use accessmod_io::scaling_up::{
    CAPACITY_FILE, CATCHMENTS_FILE, CapacityEntry, Exclusion, FACILITIES_FILE, Factor, Keep, Layer,
    Prefer, RESIDUAL_FILE, ScalingUp, ScalingUpJob, VectorSource, prepare, run,
};
use gdal::vector::{FieldValue, OGRFieldType};

/// Flat land of class 1 (walking 6 km/h: 60 s per 100 m cell) with the
/// given population.
fn fixture(dir: &Path, cols: usize, rows: usize, population: Vec<f32>) -> ScalingUpJob {
    let gt = geo_transform(X0, Y0);
    let n = cols * rows;
    write_raster::<f32>(&dir.join("dem.tif"), cols, rows, gt, vec![0.0; n], None);
    write_raster::<u8>(&dir.join("lc.tif"), cols, rows, gt, vec![1; n], Some(0.0));
    write_raster::<f32>(&dir.join("pop.tif"), cols, rows, gt, population, Some(-1.0));
    ScalingUpJob {
        schema_version: SCHEMA_VERSION,
        dem: dir.join("dem.tif"),
        land_cover: dir.join("lc.tif"),
        population: dir.join("pop.tif"),
        population_residual: None,
        facilities: None,
        scenario: vec![ScenarioEntry {
            class: 1,
            mode: TransportMode::Walking,
            speed_kmh: 6.0,
            label: None,
        }],
        method: MethodName::Isotropic,
        direction: Direction::TowardsFacilities,
        neighbourhood: Neighbourhood::Eight,
        limit_s: 120.0,
        band_s: 60.0,
        mobility_model: Default::default(),
        capacities: vec![CapacityEntry {
            min: 0.0,
            max: 1e9,
            capacity: 10.0,
            label: "any".into(),
        }],
        exclusions: vec![Exclusion {
            layer: Layer::Facilities,
            buffer_m: 100.0,
            keep: Keep::Outside,
        }],
        suitability: vec![Factor::Population {
            layer: Layer::Population,
            window_m: 0.0,
            weight: 1.0,
            prefer: Prefer::High,
        }],
        max_facilities: 2,
        max_coverage_percent: 100.0,
        max_duration_s: None,
        catchments: true,
        facility_issues: FacilityIssues::Error,
        output_dir: dir.join("out"),
    }
}

fn table_rows(out: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(out.join(CAPACITY_FILE))
        .expect("table")
        .lines()
        .skip(1)
        .map(|l| l.split(',').map(str::to_owned).collect())
        .collect()
}

#[test]
fn corridor_by_hand() {
    let dir = tempfile::tempdir().expect("tmp");
    let job = fixture(dir.path(), 5, 1, vec![1.0, 2.0, 9.0, 2.0, 1.0]);
    let out = run_job::<ScalingUp>(job, &settings(256 << 20, 1)).expect("run");
    let rows = table_rows(&out);
    assert_eq!(rows.len(), 2);
    // 1: the most populated cell (2); bands 0..2 hold 9, 4, 2: capacity 10
    // takes band 0 and a quarter of band 1 (case A).
    assert_eq!((rows[0][1].as_str(), rows[0][7].as_str()), ("1", "15"));
    assert_eq!(rows[0][17], "A");
    // 2: cells 1..3 excluded (within 100 m of facility 1); cells 0 and 4
    // tie (suitability 5000, 2.5 people each): row-major order keeps 0.
    assert_eq!((rows[1][1].as_str(), rows[1][5].as_str()), ("2", "5000"));
    assert_eq!((rows[1][7].as_str(), rows[1][17].as_str()), ("2.5", "C"));
    assert_eq!(
        read::<f64>(&out.join(RESIDUAL_FILE)),
        vec![0.0, 0.0, 0.0, 1.5, 1.0]
    );
    let points = gdal::Dataset::open(out.join(FACILITIES_FILE)).expect("facilities");
    let mut layer = points.layer(0).expect("layer");
    use gdal::vector::LayerAccess;
    let xs: Vec<f64> = layer
        .features()
        .map(|f| f.geometry().expect("point").get_point(0).0)
        .collect();
    let g = grid(5, 1);
    let mut want = vec![g.cell_centre(0, 2).0, g.cell_centre(0, 0).0];
    let mut got = xs.clone();
    got.sort_by(f64::total_cmp);
    want.sort_by(f64::total_cmp);
    assert_eq!(got, want);
}

#[test]
fn outputs_do_not_depend_on_memory() {
    let dir = tempfile::tempdir().expect("tmp");
    let (cols, rows) = (500, 400);
    let population = (0..cols * rows).map(|i| ((i * 7919) % 13) as f32).collect();
    let mut job = fixture(dir.path(), cols, rows, population);
    let g = grid(cols, rows);
    // A road across the grid and a protected area.
    let road = {
        let (x0, y0) = g.to_map(10.3, 20.7);
        let (x1, y1) = g.to_map(480.6, 350.2);
        format!("LINESTRING ({x0} {y0}, {x1} {y1})")
    };
    write_features(
        &dir.path().join("road.gpkg"),
        "road",
        EPSG,
        &[("id", OGRFieldType::OFTInteger64)],
        &[(road.as_str(), vec![Some(FieldValue::Integer64Value(1))])],
    );
    let area = cell_polygon(300.0, 50.0, 420.0, 150.0);
    write_features(
        &dir.path().join("area.gpkg"),
        "area",
        EPSG,
        &[("id", OGRFieldType::OFTInteger64)],
        &[(area.as_str(), vec![Some(FieldValue::Integer64Value(1))])],
    );
    let priority = (0..cols * rows).map(|i| (i % cols / 50) as u8).collect();
    write_raster::<u8>(
        &dir.path().join("priority.tif"),
        cols,
        rows,
        geo_transform(X0, Y0),
        priority,
        None,
    );
    let vector = |name: &str| {
        Layer::Vector(VectorSource {
            path: dir.path().join(name),
            layer: None,
            filter: None,
        })
    };
    job.limit_s = 3000.0;
    job.max_facilities = 4;
    job.exclusions.push(Exclusion {
        layer: vector("area.gpkg"),
        buffer_m: 0.0,
        keep: Keep::Outside,
    });
    job.exclusions[0].buffer_m = 2000.0;
    job.capacities = vec![
        CapacityEntry {
            min: 0.0,
            max: 5000.0,
            capacity: 2000.0,
            label: "small".into(),
        },
        CapacityEntry {
            min: 5001.0,
            max: 1e12,
            capacity: 20000.0,
            label: "large".into(),
        },
    ];
    job.suitability = vec![
        Factor::Population {
            layer: Layer::Population,
            window_m: 500.0,
            weight: 2.0,
            prefer: Prefer::High,
        },
        Factor::Distance {
            layer: Layer::Facilities,
            weight: 1.0,
            prefer: Prefer::High,
        },
        Factor::TravelTime {
            layer: vector("road.gpkg"),
            method: MethodName::Isotropic,
            direction: Direction::TowardsFacilities,
            neighbourhood: Neighbourhood::Eight,
            weight: 1.0,
            prefer: Prefer::Low,
        },
        Factor::Priority {
            layer: Layer::Raster(dir.path().join("priority.tif")),
            weight: 0.5,
            prefer: Prefer::High,
        },
    ];
    let with_dir = |out: &str| {
        let mut job = job.clone();
        job.output_dir = dir.path().join(out);
        job
    };
    let tight_budget = 40 << 20;
    let tight =
        prepare(with_dir("a"), &settings(tight_budget, 1), Monitor::QUIET).expect("prepare");
    assert!(tight.plan.resident_tiles < tight.plan.total_tiles);
    let (out_a, report_a) = run(tight, &settings(tight_budget, 1), Monitor::QUIET).expect("run a");
    assert_eq!(report_a.run.result.facilities_added, 4);
    assert!(
        report_a.execution.details.residual_store.tile_writes > 0,
        "scratch not exercised"
    );
    let out_b = run_job::<ScalingUp>(with_dir("b"), &settings(4 << 30, 4)).expect("run b");
    for name in [
        CAPACITY_FILE,
        RESIDUAL_FILE,
        CATCHMENTS_FILE,
        FACILITIES_FILE,
    ] {
        let a = std::fs::read(out_a.join(name)).expect("read a");
        let b = std::fs::read(out_b.join(name)).expect("read b");
        assert!(a == b, "{name} differs between budgets");
    }
}
