//! AccessMod 5 data archives imported as projects (`accessmod_io::am5`).
//!
//! `tests/data/am5_archive.zip` is a trimmed AccessMod 5 archive of the
//! demo project (Data manager › Archive, AccessMod 5.9): DEM, land covers,
//! a travel time, facilities, a catchment as shapefile and GeoPackage,
//! tables, analysis parameters, a directory renamed to its French label
//! (`vector_barriere_…`) and one that is no dataset (`notes`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use accessmod_io::am5;
use accessmod_io::analysis::Monitor;
use accessmod_io::archive::Limits;
use accessmod_io::class::DataClass;
use accessmod_io::fixtures::{read, read_valid, settings};
use accessmod_io::project::{Origin, Project, RunStatus, Summary};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/am5_archive.zip")
}

fn import(archive: &Path, target: &Path) -> accessmod_io::error::Result<(Project, am5::Report)> {
    am5::import_archive(
        archive,
        target,
        "From AccessMod 5",
        Limits::default(),
        &settings(64 << 20, 1),
        Monitor::QUIET,
    )
}

/// The fixture's entries, `edit`ed (`None` drops one), plus `extra`.
fn variant(
    dir: &Path,
    name: &str,
    edit: impl Fn(&str) -> Option<String>,
    extra: &[(String, Vec<u8>)],
) -> PathBuf {
    let mut zip =
        zip::ZipArchive::new(std::fs::File::open(fixture()).expect("fixture")).expect("zip");
    let path = dir.join(name);
    let mut out = zip::ZipWriter::new(std::fs::File::create(&path).expect("create"));
    let options = zip::write::SimpleFileOptions::default();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("entry");
        if entry.is_dir() {
            continue;
        }
        let Some(renamed) = edit(&entry.name().expect("name")) else {
            continue;
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read");
        out.start_file(renamed, options).expect("start");
        out.write_all(&bytes).expect("write");
    }
    for (name, bytes) in extra {
        out.start_file(name.as_str(), options).expect("start");
        out.write_all(bytes).expect("write");
    }
    out.finish().expect("finish");
    path
}

/// Bytes of the fixture entry `name`.
fn entry(name: &str) -> Vec<u8> {
    let mut zip =
        zip::ZipArchive::new(std::fs::File::open(fixture()).expect("fixture")).expect("zip");
    let mut bytes = Vec::new();
    zip.by_name(name)
        .expect("entry")
        .read_to_end(&mut bytes)
        .expect("read");
    bytes
}

#[test]
fn archive_imports_with_classes_tags_and_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (project, report) = import(&fixture(), &dir.path().join("p")).expect("import");
    assert_eq!(project.info().expect("info").name, "From AccessMod 5");
    let found: Vec<(DataClass, Vec<String>)> = project
        .datasets(None)
        .expect("datasets")
        .into_iter()
        .map(|d| (d.class, d.tags))
        .collect();
    let tags = |t: &[&str]| t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    for expected in [
        (DataClass::Dem, tags(&["dem"])),
        (DataClass::LandCover, tags(&["demo"])),
        (DataClass::LandCoverMerged, tags(&["demo"])),
        (DataClass::TravelTime, tags(&["test"])),
        (DataClass::Facility, tags(&["demo"])),
        (DataClass::Barrier, tags(&["demo", "wetland"])),
        (DataClass::Catchment, tags(&["test"])),
        (DataClass::Scenario, tags(&["demo"])),
        (DataClass::LandCoverTable, tags(&["demo"])),
    ] {
        assert!(found.contains(&expected), "{expected:?} in {found:?}");
    }
    assert_eq!(found.len(), 9, "{found:?}");
    assert_eq!(report.imported, 9);
    let skipped: Vec<&str> = report.skipped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skipped, ["config_analysis_parameters_test", "notes"]);
    assert!(report.skipped[0].reason.contains("not imported"));
    assert!(report.warnings.iter().any(|w| w.contains("10 characters")));
    assert!(report.warnings.iter().any(|w| w.contains("minutes")));
    // `raster_land_cover_merged_demo` also reads as a land cover tagged
    // `merged demo`, but its report names `rLandCoverMerged__demo`: no doubt.
    assert!(
        !report.warnings.iter().any(|w| w.contains("another class")),
        "{:?}",
        report.warnings
    );

    let datasets = project.datasets(None).expect("datasets");
    let of = |class: DataClass| datasets.iter().find(|d| d.class == class).expect("dataset");
    // The origin names the file in the archive.
    let Origin::Import { source, .. } = &of(DataClass::LandCoverMerged).origin else {
        panic!("origin");
    };
    assert_eq!(
        source,
        "am5_archive.zip/raster_land_cover_merged_demo/raster_land_cover_merged_demo.img"
    );
    // The GeoPackage was preferred to the shapefile (same directory).
    let Summary::Vector(catchment) = &of(DataClass::Catchment).summary else {
        panic!("vector");
    };
    assert!(
        catchment.geometry.contains("Polygon"),
        "{}",
        catchment.geometry
    );
    let Origin::Import { source, .. } = &of(DataClass::Catchment).origin else {
        panic!("origin");
    };
    assert!(source.ends_with(".gpkg"), "{source}");

    // Travel times: AccessMod 5 minutes, here seconds.
    let am5_minutes = {
        let path = dir.path().join("tt.img");
        std::fs::write(
            &path,
            entry("raster_travel_time_test/raster_travel_time_test.img"),
        )
        .expect("write");
        read_valid(&path)
    };
    let seconds = read_valid(
        &project
            .resolve(Path::new(&of(DataClass::TravelTime).file))
            .expect("file"),
    );
    assert_eq!(am5_minutes.len(), seconds.len());
    assert!(am5_minutes.iter().any(Option::is_some));
    for (minutes, seconds) in am5_minutes.iter().zip(&seconds) {
        assert_eq!(minutes.map(|m| m * 60.0), *seconds);
    }
}

#[test]
fn imported_inputs_equal_the_demo_and_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (project, _) = import(&fixture(), &dir.path().join("p")).expect("import");
    let settings = settings(64 << 20, 1);
    let demo =
        accessmod_io::demo::create(&dir.path().join("demo"), &settings, &|| false).expect("demo");
    let file = |p: &Project, class: DataClass| {
        let d = p
            .datasets(Some(class))
            .expect("datasets")
            .into_iter()
            .next()
            .expect("dataset");
        p.resolve(Path::new(&d.file)).expect("file")
    };
    // Same lattice (AccessMod 5 writes the CRS with its EPSG code, the demo
    // files do not).
    let (a, b) = (project.info().unwrap().grid, demo.info().unwrap().grid);
    assert_eq!(
        (a.cols, a.rows, a.geo_transform),
        (b.cols, b.rows, b.geo_transform)
    );
    assert_eq!(
        read::<f64>(&file(&project, DataClass::Dem)),
        read::<f64>(&file(&demo, DataClass::Dem))
    );
    assert_eq!(
        read::<f64>(&file(&project, DataClass::LandCover)),
        read::<f64>(&file(&demo, DataClass::LandCover))
    );

    // A travel time on the imported merged land cover and facilities.
    let land_cover = project
        .datasets(Some(DataClass::LandCoverMerged))
        .unwrap()
        .remove(0);
    let Summary::Raster(preview) = &land_cover.summary else {
        panic!("raster");
    };
    let scenario: Vec<serde_json::Value> = preview
        .stats
        .values
        .iter()
        .flatten()
        .map(|v| serde_json::json!({"class": v.value, "mode": "walking", "speed_kmh": 5.0}))
        .collect();
    assert!(!scenario.is_empty());
    let facilities = project
        .datasets(Some(DataClass::Facility))
        .unwrap()
        .remove(0);
    let dem = project.datasets(Some(DataClass::Dem)).unwrap().remove(0);
    let job = serde_json::json!({
        "schema_version": 1,
        "dem": dem.file,
        "land_cover": land_cover.file,
        "facilities": {"path": facilities.file, "id_field": "cat"},
        "scenario": scenario,
        "method": "anisotropic",
        "direction": "from_facilities",
        "neighbourhood": 8
    });
    let record = project
        .run_named("travel-time", &job, &[], false, &settings, Monitor::QUIET)
        .expect("run");
    assert_eq!(record.status, RunStatus::Done, "{:?}", record.outcome);
}

#[test]
fn misfits_refuse_the_whole_archive() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A raster off the DEM lattice: half a cell east.
    let shifted = {
        let dem = dir.path().join("dem.img");
        std::fs::write(&dem, entry("raster_dem_dem/raster_dem_dem.img")).expect("write");
        let source = gdal::Dataset::open(&dem).expect("open");
        let path = dir.path().join("shifted.tif");
        let driver = gdal::DriverManager::get_driver_by_name("GTiff").expect("driver");
        let mut copy = source
            .create_copy(&driver, &path, &Default::default())
            .expect("copy");
        let mut gt = source.geo_transform().expect("transform");
        gt[0] += gt[1] / 2.0;
        copy.set_geo_transform(&gt).expect("set");
        drop(copy);
        std::fs::read(&path).expect("read")
    };
    let off = variant(
        dir.path(),
        "off.zip",
        |n| Some(n.to_owned()),
        &[(
            "raster_population_shifted/raster_population_shifted.tif".to_owned(),
            shifted,
        )],
    );
    let target = dir.path().join("p");
    let error = import(&off, &target).expect_err("off lattice").to_string();
    assert!(error.contains("raster_population_shifted"), "{error}");
    assert!(error.contains("one by one"), "{error}");
    assert!(!target.exists());

    let two_dems = variant(
        dir.path(),
        "two.zip",
        |n| Some(n.to_owned()),
        &[(
            "raster_dem_copy/raster_dem_copy.img".to_owned(),
            entry("raster_dem_dem/raster_dem_dem.img"),
        )],
    );
    let error = import(&two_dems, &target)
        .expect_err("two DEMs")
        .to_string();
    assert!(error.contains("several DEMs"), "{error}");

    let no_dem = variant(
        dir.path(),
        "none.zip",
        |n| (!n.starts_with("raster_dem_dem/")).then(|| n.to_owned()),
        &[],
    );
    let error = import(&no_dem, &target).expect_err("no DEM").to_string();
    assert!(error.contains("no DEM"), "{error}");

    // A file GDAL would read beyond its directory is refused.
    let linked = variant(
        dir.path(),
        "vrt.zip",
        |n| Some(n.to_owned()),
        &[(
            "raster_population_vrt/raster_population_vrt.tif".to_owned(),
            b"<VRTDataset rasterXSize=\"1\" rasterYSize=\"1\"><VRTRasterBand dataType=\"Byte\" band=\"1\"><SimpleSource><SourceFilename>/etc/passwd</SourceFilename></SimpleSource></VRTRasterBand></VRTDataset>".to_vec(),
        )],
    );
    let error = import(&linked, &target).expect_err("vrt").to_string();
    assert!(error.contains("raster_population_vrt"), "{error}");
    assert!(!target.exists());

    // Nothing was left beside the target either.
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

/// The GRASS name AccessMod 5 wrote beside the data wins over the directory
/// name: a land cover in a directory renamed `raster_land_cover_merged_x`
/// is still `rLandCover__demo`, as its `r.report` says.
#[test]
fn grass_names_beside_the_data_win() {
    let dir = tempfile::tempdir().expect("tempdir");
    let renamed = variant(
        dir.path(),
        "renamed.zip",
        |n| Some(n.replace("raster_land_cover_demo/", "raster_land_cover_merged_x/")),
        &[],
    );
    let (project, report) = import(&renamed, &dir.path().join("p")).expect("import");
    let tags = |class| {
        project
            .datasets(Some(class))
            .unwrap()
            .into_iter()
            .map(|d| d.tags.join(" "))
            .collect::<Vec<_>>()
    };
    assert_eq!(tags(DataClass::LandCover), ["demo"]);
    assert_eq!(tags(DataClass::LandCoverMerged), ["demo"]);
    assert!(!report.warnings.iter().any(|w| w.contains("another class")));

    // Tables carry no name: the label decides, and the doubt is said.
    let table = variant(
        dir.path(),
        "table.zip",
        |n| Some(n.replace("table_scenario_demo/", "table_scenario_processed_demo/")),
        &[],
    );
    let (project, report) = import(&table, &dir.path().join("q")).expect("import");
    assert_eq!(
        project
            .datasets(Some(DataClass::ScenarioProcessed))
            .unwrap()
            .len(),
        1
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("another class") && w.contains("table_scenario_processed_demo")),
        "{:?}",
        report.warnings
    );
}

/// Without a GRASS name beside the data, a class this engine has not
/// (`rLandCoverBridge`) gives way to the next reading of the directory name:
/// a merged land cover tagged `bridge …`.
#[test]
fn names_of_unused_classes_read_as_the_next_label() {
    let dir = tempfile::tempdir().expect("tempdir");
    let renamed = variant(
        dir.path(),
        "bridge.zip",
        |n| {
            (!n.ends_with("_report.txt") || !n.starts_with("raster_land_cover_merged_demo/")).then(
                || {
                    n.replace(
                        "raster_land_cover_merged_demo/",
                        "raster_land_cover_merged_bridge_x/",
                    )
                },
            )
        },
        &[],
    );
    let (project, _) = import(&renamed, &dir.path().join("p")).expect("import");
    let merged = project
        .datasets(Some(DataClass::LandCoverMerged))
        .expect("datasets");
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].tags, ["bridge", "x"]);
}

/// A dataset directory without its main file (a shapefile without
/// `.shp`) refuses the archive: whole or nothing.
#[test]
fn damaged_datasets_refuse_the_archive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let damaged = variant(
        dir.path(),
        "damaged.zip",
        |n| (!n.ends_with("vector_facility_demo.shp")).then(|| n.to_owned()),
        &[],
    );
    let target = dir.path().join("p");
    let error = import(&damaged, &target).expect_err("damaged").to_string();
    assert!(error.contains("vector_facility_demo"), "{error}");
    assert!(error.contains("no vector file"), "{error}");
    assert!(!target.exists());
}

#[test]
fn archives_zipped_from_a_folder_import_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let nested = variant(
        dir.path(),
        "nested.zip",
        |n| Some(format!("export/{n}")),
        &[],
    );
    let (project, _) = import(&nested, &dir.path().join("p")).expect("import");
    assert_eq!(project.datasets(None).unwrap().len(), 9);
}

/// The whole archive of the AccessMod 5 demo project, when
/// `ACCESSMOD_AM5_ARCHIVE` names one: every dataset but the analysis
/// parameters imports, the demo inputs among them.
#[test]
fn full_archive_imports() {
    let Some(archive) = std::env::var_os("ACCESSMOD_AM5_ARCHIVE").map(PathBuf::from) else {
        eprintln!("skipped: set ACCESSMOD_AM5_ARCHIVE to an AccessMod 5 data archive");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let (project, report) = import(&archive, &dir.path().join("p")).expect("import");
    eprintln!("{}", report.summary());
    let datasets = project.datasets(None).unwrap();
    assert_eq!(datasets.len(), report.imported);
    for class in [
        DataClass::Dem,
        DataClass::LandCover,
        DataClass::LandCoverMerged,
        DataClass::Population,
        DataClass::Facility,
        DataClass::Road,
        DataClass::Barrier,
        DataClass::Zone,
        DataClass::Scenario,
        DataClass::LandCoverTable,
        DataClass::TravelTime,
        DataClass::Catchment,
    ] {
        assert!(
            datasets.iter().any(|d| d.class == class),
            "no {class:?} imported"
        );
    }
    assert!(
        report.skipped.iter().all(|s| s.name.starts_with("config_")),
        "{:?}",
        report.skipped
    );
}
