//! Project Creator from local files (`accessmod_io::creator`).
//!
//! `tests/data/creator/` holds real sources cut around Juba, South Sudan
//! (31.58–31.635° E, 4.83–4.885° N): Copernicus GLO-90 in two tiles (the
//! seam is crossed), ESA WorldCover 2021, WorldPop R2025A, and the HOT OSM
//! exports (roads, waterways, health facilities; ODbL).

use std::path::{Path, PathBuf};

use accessmod_io::analysis::Monitor;
use accessmod_io::class::DataClass;
use accessmod_io::creator::{self, Area, Crs, FacilitySource, Recipe, Sources};
use accessmod_io::fixtures::{read_valid, settings, write_features};
use accessmod_io::project::RunStatus;
use gdal::vector::{FieldValue, OGRFieldType};
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
            land_cover_worldcover: true,
            population: vec![data("worldpop.tif")],
            roads: Some(data("roads.gpkg")),
            ferries: vec![],
            waterways: Some(data("waterways.gpkg")),
            facilities: Some(FacilitySource::Osm(data("health_facilities.gpkg"))),
            urls: vec![],
            attributions: vec![],
            warnings: vec![],
        },
        default_scenarios: true,
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
    assert!(manifest.ready, "{:?}", manifest.not_ready);
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
    // Exactly what was asked: every layer, the merged land cover and its
    // table, the three scenario tables.
    let mut got: Vec<String> = project
        .datasets(None)
        .unwrap()
        .iter()
        .map(|d| format!("{} {}", d.class.code(), d.tags.join(" ")))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            "rDem dem",
            "rLandCover worldcover",
            "rLandCoverMerged creator",
            "rPopulation worldpop",
            "tLandCover creator",
            "tScenario creator bicycle",
            "tScenario creator motorized",
            "tScenario creator walking",
            "vBarrier osm rivers",
            "vBarrier outside region",
            "vFacility osm",
            "vRoad osm",
            "vZone area",
        ],
    );
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
    // Roads cross rivers: the rivers sit below the roads in the stack. The
    // same merge with the rivers above the roads (every barrier closing
    // everything, as before 2026-11-18) keeps fewer road cells.
    let road_cells =
        |cells: &[Option<f64>]| cells.iter().flatten().filter(|v| **v > 1000.0).count();
    let merge_run = project
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.analysis == "merge-land-cover")
        .expect("merge run");
    let mut closed = merge_run.job.clone();
    let stack = closed["stack"].as_array_mut().expect("stack");
    let road = stack
        .iter()
        .position(|i| i["type"] == "road")
        .expect("road item");
    let river = stack
        .iter()
        .rposition(|i| i["type"] == "barrier")
        .expect("river item");
    assert!(road < river, "rivers below the roads: {stack:?}");
    let item = stack.remove(river);
    stack.insert(road, item);
    let closed_run = project
        .run_named(
            "merge-land-cover",
            &closed,
            &["rivers-on-top".to_owned()],
            false,
            &s,
            Monitor::QUIET,
        )
        .expect("merge with rivers on top");
    let closed_merged = project.datasets(Some(DataClass::LandCoverMerged)).unwrap();
    let closed_merged = closed_merged
        .iter()
        .find(|d| closed_run.datasets.contains(&d.id))
        .expect("merged of the rivers-on-top run");
    let closed_cells = read_valid(&project.resolve(Path::new(&closed_merged.file)).unwrap());
    assert!(
        road_cells(&cells) > road_cells(&closed_cells),
        "roads over rivers: {} road cells, {} with rivers on top",
        road_cells(&cells),
        road_cells(&closed_cells)
    );
    // The scenario tables hold what the saved jobs hold, as the client
    // reads them: water a barrier (0), a primary road per scenario (the
    // fixture has no motorway: its 0.1 km/h is checked on the preset).
    let table = |name: &str| -> Vec<Vec<String>> {
        let d = project
            .datasets(Some(DataClass::Scenario))
            .unwrap()
            .into_iter()
            .find(|d| d.tags.iter().any(|t| t == name))
            .unwrap_or_else(|| panic!("{name} table"));
        let file = std::fs::File::open(project.resolve(Path::new(&d.file)).unwrap()).unwrap();
        let mut rows = Vec::new();
        accessmod_io::output::read_csv(file, 1 << 20, |r| {
            rows.push(r.to_vec());
            Ok(())
        })
        .unwrap();
        rows
    };
    let row = |rows: &[Vec<String>], class: &str| -> Vec<String> {
        rows.iter()
            .find(|r| r[0] == class)
            .cloned()
            .unwrap_or_default()
    };
    let walking = table("walking");
    assert_eq!(walking[0], ["class", "label", "speed", "mode"]);
    assert_eq!(row(&walking, "80")[2..], ["0.0", "WALKING"]);
    assert_eq!(row(&walking, "1002")[2..], ["5.0", "WALKING"]);
    assert_eq!(row(&table("bicycle"), "1002")[2..], ["15.0", "BICYCLING"]);
    assert_eq!(row(&table("motorized"), "1002")[2..], ["60.0", "MOTORIZED"]);
    // The three saved travel-time jobs run; off the roads people walk, so
    // only the motorized one has a motorized class, and it reaches the
    // population sooner than walking.
    let jobs = project.jobs().expect("jobs");
    let mut means = Vec::new();
    for name in ["walking", "bicycle", "motorized"] {
        let job = jobs
            .iter()
            .find(|j| {
                j.analysis == "travel-time"
                    && j.name == format!("Travel time, {name} (Project Creator)")
            })
            .unwrap_or_else(|| panic!("saved {name} job"));
        let modes: Vec<&str> = job.document["scenario"]
            .as_array()
            .expect("scenario")
            .iter()
            .filter_map(|e| e["mode"].as_str())
            .collect();
        assert_eq!(
            modes.contains(&"motorized"),
            name == "motorized",
            "{name}: {modes:?}"
        );
        let run = project
            .run_named("travel-time", &job.document, &[], false, &s, Monitor::QUIET)
            .expect("run");
        assert_eq!(run.status, RunStatus::Done, "{:?}", run.outcome);
        let time = run
            .datasets
            .iter()
            .filter_map(|&id| project.dataset(id).ok())
            .find(|d| d.class == DataClass::TravelTime)
            .expect("travel time");
        let cells: Vec<f64> = read_valid(&project.resolve(Path::new(&time.file)).unwrap())
            .into_iter()
            .flatten()
            .collect();
        means.push(cells.iter().sum::<f64>() / cells.len() as f64);
    }
    assert!(
        means[2] < means[1] && means[1] < means[0],
        "mean times {means:?}"
    );
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

/// Refusals that must not leave a silent hole: an invalid polygon (the
/// outside barrier would be lost), a missing DEM tile.
#[test]
fn invalid_areas_and_missing_tiles_are_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    let mut bowtie = recipe(0.0, Crs::Auto);
    bowtie.area = Area::Geojson(
        json!({"type": "Polygon", "coordinates": [[[31.6, 4.85], [31.615, 4.865], [31.615, 4.85], [31.6, 4.865], [31.6, 4.85]]]}),
    );
    let error = creator::build(&bowtie, &dir.path().join("a"), &s, Monitor::QUIET)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a valid polygon"), "{error}");

    let mut half = recipe(0.0, Crs::Auto);
    half.sources.dem = vec![data("dem_west.tif")];
    let error = creator::build(&half, &dir.path().join("b"), &s, Monitor::QUIET)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("DEM") && error.contains("missing tiles"),
        "{error}"
    );

    for name in ["a", "b"] {
        assert!(!dir.path().join(name).exists());
    }
}

/// Merged land cover cell at WGS 84 `(lon, lat)` of `project` (UTM 36N).
fn merged_at(project: &accessmod_io::project::Project, lon: f64, lat: f64) -> Option<f64> {
    use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
    let merged = project
        .datasets(Some(DataClass::LandCoverMerged))
        .unwrap()
        .remove(0);
    let cells = read_valid(&project.resolve(Path::new(&merged.file)).unwrap());
    let info = project.info().unwrap();
    let gt = info.grid.geo_transform;
    let mut wgs = SpatialRef::from_epsg(4326).unwrap();
    wgs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    let mut utm = SpatialRef::from_epsg(32636).unwrap();
    utm.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    let t = CoordTransform::new(&wgs, &utm).unwrap();
    let (mut x, mut y) = ([lon], [lat]);
    t.transform_coords(&mut x, &mut y, &mut []).unwrap();
    let col = ((x[0] - gt[0]) / gt[1]).floor();
    let row = ((y[0] - gt[3]) / gt[5]).floor();
    if col < 0.0 || row < 0.0 || col >= info.grid.cols as f64 || row >= info.grid.rows as f64 {
        return None;
    }
    cells[row as usize * info.grid.cols + col as usize]
}

/// Holes stay impassable, islands stay apart; population split in two
/// country files (no overlap) gives the same total as one file.
#[test]
fn holes_islands_and_two_population_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    let square = |w: f64, s: f64, e: f64, n: f64| json!([[w, s], [e, s], [e, n], [w, n], [w, s]]);
    // Without hole nor island first: the points checked below are passable.
    let mut plain = recipe(0.0, Crs::Epsg(32636));
    plain.area = Area::Geojson(
        json!({"type": "Polygon", "coordinates": [square(31.582, 4.832, 31.605, 4.862)]}),
    );
    let (base, _) =
        creator::build(&plain, &dir.path().join("plain"), &s, Monitor::QUIET).expect("plain");
    let (hole, island, between) = ((31.594, 4.852), (31.585, 4.836), (31.586, 4.843));
    for (lon, lat) in [hole, island, between] {
        assert!(
            merged_at(&base, lon, lat).is_some(),
            "{lon} {lat} not passable to begin with"
        );
    }
    // A square with a hole, and a separate island south-west of it.
    let mut r = recipe(0.0, Crs::Epsg(32636));
    r.area = Area::Geojson(json!({"type": "MultiPolygon", "coordinates": [
        [square(31.589, 4.846, 31.603, 4.86), square(31.592, 4.85, 31.596, 4.854)],
        [square(31.583, 4.833, 31.587, 4.839)]
    ]}));
    // Population in two halves, west and east of a column of the source.
    let whole = gdal::Dataset::open(data("worldpop.tif")).unwrap();
    let (cols, rows) = whole.raster_size();
    let gt = whole.geo_transform().unwrap();
    let band = whole.rasterband(1).unwrap();
    let nodata = band.no_data_value();
    let mut halves = Vec::new();
    for (name, c0, c1) in [("west.tif", 0, cols / 2), ("east.tif", cols / 2, cols)] {
        let mut values = band
            .read_as::<f32>((c0 as isize, 0), (c1 - c0, rows), (c1 - c0, rows), None)
            .unwrap();
        let path = dir.path().join(name);
        let driver = gdal::DriverManager::get_driver_by_name("GTiff").unwrap();
        let mut out = driver
            .create_with_band_type::<f32, _>(&path, c1 - c0, rows, 1)
            .unwrap();
        out.set_geo_transform(&[gt[0] + c0 as f64 * gt[1], gt[1], 0.0, gt[3], 0.0, gt[5]])
            .unwrap();
        out.set_spatial_ref(&whole.spatial_ref().unwrap()).unwrap();
        let mut b = out.rasterband(1).unwrap();
        b.set_no_data_value(nodata).unwrap();
        b.write((0, 0), (c1 - c0, rows), &mut values).unwrap();
        halves.push(path);
    }
    let (one, m1) =
        creator::build(&r, &dir.path().join("one"), &s, Monitor::QUIET).expect("one file");
    r.sources.population = halves;
    let (_, m2) =
        creator::build(&r, &dir.path().join("two"), &s, Monitor::QUIET).expect("two files");
    let (p1, p2) = (m1.population.unwrap(), m2.population.unwrap());
    assert!((p1.grid - p2.grid).abs() < 1e-6 * p1.grid, "{p1:?} {p2:?}");
    assert!(
        merged_at(&one, hole.0, hole.1).is_none(),
        "the hole is a barrier"
    );
    assert!(
        merged_at(&one, island.0, island.1).is_some(),
        "the island is passable"
    );
    assert!(
        merged_at(&one, between.0, between.1).is_none(),
        "between them is a barrier"
    );
}

/// Only the DEM is required: without facilities, or without land cover,
/// the project is created and says what it lacks to compute.
#[test]
fn projects_without_facilities_or_land_cover_say_what_they_lack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    let mut no_facilities = recipe(0.0, Crs::Auto);
    no_facilities.sources.facilities = None;
    let (project, manifest) =
        creator::build(&no_facilities, &dir.path().join("a"), &s, Monitor::QUIET)
            .expect("no facilities");
    assert!(!manifest.ready);
    assert!(
        manifest
            .not_ready
            .iter()
            .any(|r| r.contains("no health facility")),
        "{:?}",
        manifest.not_ready
    );
    assert!(project.jobs().unwrap().is_empty());
    assert!(
        !project
            .datasets(Some(DataClass::LandCoverMerged))
            .unwrap()
            .is_empty()
    );

    let mut dem_only = recipe(0.0, Crs::Auto);
    dem_only.sources = Sources {
        dem: vec![data("dem_west.tif"), data("dem_east.tif")],
        land_cover: vec![],
        land_cover_worldcover: true,
        population: vec![],
        roads: None,
        ferries: vec![],
        waterways: None,
        facilities: None,
        urls: vec![],
        attributions: vec![],
        warnings: vec![],
    };
    let (project, manifest) =
        creator::build(&dem_only, &dir.path().join("b"), &s, Monitor::QUIET).expect("DEM only");
    assert!(!manifest.ready);
    assert!(
        manifest
            .not_ready
            .iter()
            .any(|r| r.contains("no land cover"))
    );
    let classes: Vec<DataClass> = project
        .datasets(None)
        .unwrap()
        .iter()
        .map(|d| d.class)
        .collect();
    assert_eq!(classes.len(), 3, "{classes:?}"); // DEM, outside barrier, zones
}

/// A facility list of your own is kept whole (every feature in the region,
/// every field), with its own identifier; no OpenStreetMap warning.
#[test]
fn facility_lists_are_kept_whole() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    let mut osm = recipe(500.0, Crs::Auto);
    let (_, from_osm) =
        creator::build(&osm, &dir.path().join("osm"), &s, Monitor::QUIET).expect("osm");
    osm.sources.facilities = Some(FacilitySource::List {
        path: data("health_facilities.gpkg"),
        id_field: "osm_id".to_owned(),
    });
    // Declined default scenarios: no scenario table, no saved job.
    osm.default_scenarios = false;
    let (project, from_list) =
        creator::build(&osm, &dir.path().join("list"), &s, Monitor::QUIET).expect("list");
    assert!(
        project
            .datasets(Some(DataClass::Scenario))
            .unwrap()
            .is_empty()
    );
    assert!(project.jobs().unwrap().is_empty());
    let count = |m: &creator::Manifest, key: &str| {
        m.features
            .iter()
            .find(|(k, _)| k.starts_with(key))
            .map(|(_, n)| *n)
    };
    let (osm_n, list_n) = (
        count(&from_osm, "facilities").unwrap(),
        count(&from_list, "facilities").unwrap(),
    );
    assert!(
        list_n > osm_n,
        "the list keeps pharmacies too: {list_n} vs {osm_n}"
    );
    assert!(
        from_osm
            .warnings
            .iter()
            .any(|w| w.contains("OpenStreetMap"))
    );
    assert!(
        !from_list
            .warnings
            .iter()
            .any(|w| w.contains("OpenStreetMap"))
    );
    assert!(from_list.ready, "{:?}", from_list.not_ready);
    let facilities = project
        .datasets(Some(DataClass::Facility))
        .unwrap()
        .remove(0);
    let accessmod_io::project::Summary::Vector(v) = facilities.summary else {
        panic!("vector")
    };
    assert!(v.fields.iter().any(|f| f == "amenity"), "{:?}", v.fields);
    let mut bad = osm.clone();
    bad.sources.facilities = Some(FacilitySource::List {
        path: data("health_facilities.gpkg"),
        id_field: "nope".to_owned(),
    });
    assert!(
        creator::build(&bad, &dir.path().join("bad"), &s, Monitor::QUIET)
            .unwrap_err()
            .to_string()
            .contains("no field")
    );
}

/// Ferries (raw-data API tiles, HOT layout) join the roads as class 1008,
/// above the rivers: a ferry crosses one. A line cut at a tile edge comes
/// in two pieces, both kept; the same piece in two tiles is kept once.
#[test]
fn ferries_join_the_roads_and_cross_rivers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    let fields = [
        ("osm_id", OGRFieldType::OFTInteger64),
        ("osm_type", OGRFieldType::OFTString),
        ("name", OGRFieldType::OFTString),
        ("route", OGRFieldType::OFTString),
        ("highway", OGRFieldType::OFTString),
    ];
    let ferry = |wkt: &'static str| {
        (
            wkt,
            vec![
                Some(FieldValue::Integer64Value(42)),
                Some(FieldValue::StringValue("ways_line".into())),
                Some(FieldValue::StringValue("Juba crossing".into())),
                Some(FieldValue::StringValue("ferry".into())),
                None,
            ],
        )
    };
    // Across the river (31.616–31.621° E at 4.863–4.875° N), cut at 31.618.
    let west = "LINESTRING (31.612 4.868, 31.618 4.868)";
    let east = "LINESTRING (31.618 4.868, 31.624 4.868)";
    let tiles = [
        dir.path().join("tile_1.gpkg"),
        dir.path().join("tile_2.gpkg"),
    ];
    write_features(&tiles[0], "ferries", 4326, &fields, &[ferry(west)]);
    write_features(
        &tiles[1],
        "ferries",
        4326,
        &fields,
        &[ferry(east), ferry(west)],
    );

    let without = recipe(500.0, Crs::Auto);
    let mut with = without.clone();
    with.sources.ferries = tiles.to_vec();
    let (plain, _) =
        creator::build(&without, &dir.path().join("plain"), &s, Monitor::QUIET).expect("plain");
    let (project, manifest) =
        creator::build(&with, &dir.path().join("ferries"), &s, Monitor::QUIET).expect("ferries");
    let count = |key: &str| {
        manifest
            .features
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, n)| *n)
    };
    assert_eq!(count("ferries"), Some(2), "{:?}", manifest.features);
    let roads = count("roads").expect("roads");
    assert!(roads > 0);
    assert!(manifest.files.iter().any(|f| f.name == "tile_2.gpkg"));

    let merged = |p: &accessmod_io::project::Project| {
        let d = p
            .datasets(Some(DataClass::LandCoverMerged))
            .unwrap()
            .remove(0);
        read_valid(&p.resolve(Path::new(&d.file)).unwrap())
    };
    let (before, after) = (merged(&plain), merged(&project));
    assert_eq!(before.len(), after.len(), "same grid");
    let ferry_cells = after.iter().filter(|v| **v == Some(1008.0)).count();
    assert!(ferry_cells > 0);
    // Cells the river closed are now open by ferry.
    let crossed = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| b.is_none() && **a == Some(1008.0))
        .count();
    assert!(crossed > 0, "{ferry_cells} ferry cells, none on the river");

    // Every scenario crosses by boat at 20 km/h.
    for name in ["walking", "bicycle", "motorized"] {
        let d = project
            .datasets(Some(DataClass::Scenario))
            .unwrap()
            .into_iter()
            .find(|d| d.tags.iter().any(|t| t == name))
            .unwrap_or_else(|| panic!("{name} table"));
        let text = std::fs::read_to_string(project.resolve(Path::new(&d.file)).unwrap()).unwrap();
        assert!(
            text.lines().any(|l| l == "1008,Ferry,20.0,MOTORIZED"),
            "{name}: {text}"
        );
    }
}

/// A copy of fixture `name` in `dir` with `value` in every pixel whose
/// centre lies in the WGS 84 box `(west, south, east, north)`.
fn patched(dir: &Path, name: &str, bbox: (f64, f64, f64, f64), value: f64) -> PathBuf {
    use gdal::raster::Buffer;
    use gdal::{Dataset, DatasetOptions, GdalOpenFlags};
    let target = dir.join(name);
    std::fs::copy(data(name), &target).expect("copy");
    let ds = Dataset::open_ex(
        &target,
        DatasetOptions {
            open_flags: GdalOpenFlags::GDAL_OF_RASTER | GdalOpenFlags::GDAL_OF_UPDATE,
            ..Default::default()
        },
    )
    .expect("open for update");
    let gt = ds.geo_transform().expect("geotransform");
    let (cols, rows) = ds.raster_size();
    let mut band = ds.rasterband(1).expect("band");
    let mut values = band
        .read_as::<f64>((0, 0), (cols, rows), (cols, rows), None)
        .expect("read")
        .into_shape_and_vec()
        .1;
    for row in 0..rows {
        for col in 0..cols {
            let x = gt[0] + (col as f64 + 0.5) * gt[1];
            let y = gt[3] + (row as f64 + 0.5) * gt[5];
            if x > bbox.0 && x < bbox.2 && y > bbox.1 && y < bbox.3 {
                values[row * cols + col] = value;
            }
        }
    }
    let mut buffer = Buffer::new((cols, rows), values);
    band.write((0, 0), (cols, rows), &mut buffer)
        .expect("write");
    target
}

/// WorldCover leaves open sea far from the coast without data; the DEM
/// says 0 there. Such cells are spared by the missing-tile check (a
/// warning, cells left NoData); land cover NoData over land is not.
#[test]
fn open_sea_without_land_cover_is_a_warning_not_a_missing_tile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s = settings(64 << 20, 1);
    // As at sea: WorldCover's NoData starts offshore, where the DEM is
    // already 0 (a few DEM cells around the hole).
    let hole_box = (31.611, 4.849, 31.619, 4.857);
    let sea = (31.608, 4.846, 31.622, 4.86);
    let land_cover = patched(dir.path(), "worldcover.tif", hole_box, 0.0);

    let mut hole = recipe(0.0, Crs::Auto);
    hole.sources.land_cover = vec![land_cover.clone()];
    let error = creator::build(&hole, &dir.path().join("hole"), &s, Monitor::QUIET)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("land cover") && error.contains("missing tiles"),
        "{error}"
    );

    let mut open_sea = hole.clone();
    open_sea.sources.dem = vec![
        data("dem_west.tif"),
        patched(dir.path(), "dem_east.tif", sea, 0.0),
    ];
    let (project, manifest) =
        creator::build(&open_sea, &dir.path().join("sea"), &s, Monitor::QUIET).expect("sea");
    let warning = manifest
        .warnings
        .iter()
        .find(|w| w.contains("open sea"))
        .unwrap_or_else(|| panic!("{:?}", manifest.warnings));
    assert!(
        warning.contains("without land cover (DEM at 0)"),
        "{warning}"
    );
    // The sea stays NoData in the merged land cover.
    assert_eq!(merged_at(&project, 31.615, 4.853), None);
    assert!(merged_at(&project, 31.6, 4.86).is_some());
}
