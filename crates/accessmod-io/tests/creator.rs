//! Project Creator from local files (`accessmod_io::creator`).
//!
//! `tests/data/creator/` holds real sources cut around Juba, South Sudan
//! (31.58–31.635° E, 4.83–4.885° N): Copernicus GLO-90 in two tiles (the
//! seam is crossed), ESA WorldCover 2021, WorldPop R2025A, and the HOT OSM
//! exports (roads, waterways, health facilities; ODbL).

use std::path::{Path, PathBuf};

use accessmod_io::analysis::Monitor;
use accessmod_io::class::DataClass;
use accessmod_io::creator::{self, Area, Crs, Recipe, Sources};
use accessmod_io::fixtures::{read_valid, settings};
use accessmod_io::project::RunStatus;
use serde_json::json;

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/creator")
        .join(name)
}

fn area() -> serde_json::Value {
    json!({
        "type": "FeatureCollection",
        "features": [
            {"type": "Feature", "properties": {"name": "Juba west"}, "geometry": {"type": "Polygon", "coordinates": [[[31.595, 4.845], [31.6075, 4.845], [31.6075, 4.87], [31.595, 4.87], [31.595, 4.845]]]}},
            {"type": "Feature", "properties": {"name": "Juba east"}, "geometry": {"type": "Polygon", "coordinates": [[[31.6075, 4.845], [31.62, 4.845], [31.62, 4.87], [31.6075, 4.87], [31.6075, 4.845]]]}}
        ]
    })
}

fn recipe(buffer_m: f64, crs: Crs) -> Recipe {
    Recipe {
        schema_version: 1,
        name: "Juba".to_owned(),
        area: Area::Geojson(area()),
        buffer_m,
        resolution_m: 100.0,
        crs,
        sources: Sources {
            dem: vec![data("dem_west.tif"), data("dem_east.tif")],
            land_cover: vec![data("worldcover.tif")],
            population: vec![data("worldpop.tif")],
            roads: Some(data("roads.gpkg")),
            waterways: Some(data("waterways.gpkg")),
            facilities: Some(data("health_facilities.gpkg")),
        },
    }
}

#[test]
fn a_region_becomes_a_project_that_computes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    let (project, manifest) = creator::build(
        &recipe(500.0, Crs::Auto),
        &dir.path().join("p"),
        &s,
        Monitor::QUIET,
    )
    .expect("build");
    eprintln!("{manifest:#?}");
    // UTM 36N fits; distances within 0.1 %.
    assert!(
        manifest.crs_name.starts_with("UTM zone 36N"),
        "{}",
        manifest.crs_name
    );
    assert!(manifest.distortion.distance < 1e-3);
    // Population kept over the footprint.
    let pop = manifest.population.expect("population");
    assert!(pop.source > 0.0);
    assert!(((pop.grid - pop.source) / pop.source).abs() < creator::POPULATION_TOLERANCE);
    // Every dataset, and the merged land cover.
    let classes: Vec<DataClass> = project
        .datasets(None)
        .unwrap()
        .iter()
        .map(|d| d.class)
        .collect();
    for class in [
        DataClass::Dem,
        DataClass::LandCover,
        DataClass::Population,
        DataClass::Road,
        DataClass::Barrier,
        DataClass::Facility,
        DataClass::Zone,
        DataClass::LandCoverMerged,
    ] {
        assert!(classes.contains(&class), "no {class:?} in {classes:?}");
    }
    // Outside the buffered region nothing can be crossed: the merged land
    // cover is NoData at the grid corners (outside the buffer), set inside.
    let merged = project
        .datasets(Some(DataClass::LandCoverMerged))
        .unwrap()
        .remove(0);
    let cells = read_valid(&project.resolve(Path::new(&merged.file)).unwrap());
    let info = project.info().unwrap();
    let (cols, rows) = (info.grid.cols, info.grid.rows);
    assert!(cells[0].is_none() && cells[cols * rows - 1].is_none());
    assert!(cells[(rows / 2) * cols + cols / 2].is_some());
    // Roads are in (class + 1000).
    assert!(cells.iter().flatten().any(|v| *v > 1000.0));
    // The saved travel-time job runs.
    let jobs = project.jobs().expect("jobs");
    let job = jobs
        .iter()
        .find(|j| j.analysis == "travel-time")
        .expect("saved travel-time job");
    let run = project
        .run_named("travel-time", &job.document, &[], false, &s, Monitor::QUIET)
        .expect("run");
    assert_eq!(run.status, RunStatus::Done, "{:?}", run.outcome);
    // The manifest is kept in the project.
    assert!(
        manifest
            .files
            .iter()
            .any(|f| f.name == "roads.gpkg" && f.sha256.len() == 64)
    );
}

#[test]
fn outputs_do_not_depend_on_memory_or_threads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut digests = Vec::new();
    for (i, (memory, threads)) in [(32 << 20, 1), (512 << 20, 4)].into_iter().enumerate() {
        let (project, _) = creator::build(
            &recipe(0.0, Crs::Auto),
            &dir.path().join(format!("p{i}")),
            &settings(memory, threads),
            Monitor::QUIET,
        )
        .expect("build");
        let mut sums: Vec<(String, String)> = project
            .datasets(None)
            .unwrap()
            .into_iter()
            .map(|d| (format!("{:?} {:?}", d.class, d.tags), d.sha256))
            .collect();
        sums.sort();
        digests.push(sums);
    }
    assert_eq!(digests[0], digests[1]);
}

#[test]
fn reference_lattices_and_refusals() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    // A national CRS by EPSG code.
    let (project, manifest) = creator::build(
        &recipe(0.0, Crs::Epsg(32636)),
        &dir.path().join("epsg"),
        &s,
        Monitor::QUIET,
    )
    .expect("EPSG");
    assert_eq!(manifest.crs_name, "EPSG:32636");
    // The lattice of a reference raster: its origin modulo the cell size.
    let dem = project.datasets(Some(DataClass::Dem)).unwrap().remove(0);
    let reference = project.resolve(Path::new(&dem.file)).unwrap();
    let shifted = dir.path().join("reference.tif");
    {
        let source = gdal::Dataset::open(&reference).unwrap();
        let driver = gdal::DriverManager::get_driver_by_name("GTiff").unwrap();
        let mut copy = source
            .create_copy(&driver, &shifted, &Default::default())
            .unwrap();
        let mut gt = source.geo_transform().unwrap();
        gt[0] += 37.0;
        gt[3] -= 11.0;
        copy.set_geo_transform(&gt).unwrap();
    }
    let (other, _) = creator::build(
        &recipe(0.0, Crs::Reference(shifted.clone())),
        &dir.path().join("reference"),
        &s,
        Monitor::QUIET,
    )
    .expect("reference");
    let gt = other.info().unwrap().grid.geo_transform;
    let offset = |a: f64, b: f64| ((a - b) / 100.0 - ((a - b) / 100.0).round()).abs();
    let ref_gt = gdal::Dataset::open(&shifted)
        .unwrap()
        .geo_transform()
        .unwrap();
    assert!(offset(gt[0], ref_gt[0]) < 1e-9 && offset(gt[3], ref_gt[3]) < 1e-9);
    // Another resolution than the reference's is refused.
    let mut wrong = recipe(0.0, Crs::Reference(shifted));
    wrong.resolution_m = 250.0;
    assert!(creator::build(&wrong, &dir.path().join("wrong"), &s, Monitor::QUIET).is_err());
    // Web Mercator distorts too much at any latitude off the equator? Here
    // near 5° N it is within limits, but a geographic CRS is refused.
    assert!(
        creator::build(
            &recipe(0.0, Crs::Epsg(4326)),
            &dir.path().join("geo"),
            &s,
            Monitor::QUIET
        )
        .is_err()
    );
    // A point is not an area.
    let mut point = recipe(0.0, Crs::Auto);
    point.area = Area::Geojson(json!({"type": "Point", "coordinates": [31.6, 4.85]}));
    assert!(creator::build(&point, &dir.path().join("point"), &s, Monitor::QUIET).is_err());
    // Nothing is left behind by failures.
    for name in ["wrong", "geo", "point"] {
        assert!(!dir.path().join(name).exists(), "{name}");
    }
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
