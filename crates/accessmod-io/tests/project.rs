//! Projects: creation from a DEM, imports, saved jobs, runs registering
//! their outputs, paths confined to the project, crash recovery.

use std::path::Path;

use accessmod_io::analysis::Monitor;
use accessmod_io::class::DataClass;
use accessmod_io::fixtures::{
    EPSG, X0, Y0, geo_transform, grid, read, settings, write_features, write_raster,
};
use accessmod_io::project::{ImportOptions, Origin, Project, RunStatus, Summary};
use accessmod_io::travel_time::TravelTime;
use gdal::vector::{FieldValue, OGRFieldType};
use serde_json::json;

const COLS: usize = 6;
const ROWS: usize = 4;

/// Source files: DEM, land cover (class 1, one NoData cell), facilities.
fn sources(dir: &Path) {
    write_raster::<f32>(
        &dir.join("dem.tif"),
        COLS,
        ROWS,
        geo_transform(X0, Y0),
        vec![10.0; COLS * ROWS],
        Some(-9999.0),
    );
    let mut classes = vec![1u8; COLS * ROWS];
    classes[COLS * ROWS - 1] = 0;
    write_raster(
        &dir.join("lc.tif"),
        COLS,
        ROWS,
        geo_transform(X0, Y0),
        classes,
        Some(0.0),
    );
    let (x, y) = grid(COLS, ROWS).cell_centre(1, 1);
    write_features(
        &dir.join("hf.gpkg"),
        "facilities",
        EPSG,
        &[
            ("hf_id", OGRFieldType::OFTInteger),
            ("name", OGRFieldType::OFTString),
        ],
        &[(
            &format!("POINT ({x} {y})"),
            vec![
                Some(FieldValue::IntegerValue(7)),
                Some(FieldValue::StringValue("Central".into())),
            ],
        )],
    );
}

fn tags(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

fn project_with_inputs(dir: &Path) -> Project {
    sources(dir);
    let settings = settings(64 << 20, 1);
    let project = Project::create(
        &dir.join("p"),
        "Demo",
        "test project",
        &dir.join("dem.tif"),
        &settings,
        &|| false,
    )
    .expect("create");
    project
        .import(
            &dir.join("lc.tif"),
            DataClass::LandCoverMerged,
            &tags(&["base"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("land cover");
    project
        .import(
            &dir.join("hf.gpkg"),
            DataClass::Facility,
            &tags(&["health", "centres"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("facilities");
    project
}

fn travel_time_job() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "dem": "datasets/1.tif",
        "land_cover": "datasets/2.tif",
        "facilities": {"path": "datasets/3.fgb", "id_field": "hf_id"},
        "scenario": [{"class": 1, "mode": "motorized", "speed_kmh": 10.0}],
        "method": "isotropic",
        "direction": "from_facilities",
        "neighbourhood": 8
    })
}

#[test]
fn create_import_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let info = project.info().expect("info");
    assert_eq!(info.name, "Demo");
    assert_eq!((info.grid.cols, info.grid.rows), (COLS, ROWS));
    assert_eq!(info.dem, 1);

    let all = project.datasets(None).expect("datasets");
    let classes: Vec<DataClass> = all.iter().map(|d| d.class).collect();
    assert_eq!(
        classes,
        [
            DataClass::Dem,
            DataClass::LandCoverMerged,
            DataClass::Facility
        ]
    );
    assert_eq!(all[1].file, "datasets/2.tif");
    assert_eq!(all[2].tags, ["health", "centres"]);
    assert!(matches!(&all[1].origin, Origin::Import { source, .. } if source.ends_with("lc.tif")));

    // Land cover: same cells and NoData as the source; class counts.
    let copied: Vec<u8> = read(&dir.path().join("p/datasets/2.tif"));
    let source: Vec<u8> = read(&dir.path().join("lc.tif"));
    assert_eq!(copied, source);
    let Summary::Raster(preview) = &all[1].summary else {
        panic!("raster summary");
    };
    assert_eq!(preview.stats.valid_cells, (COLS * ROWS - 1) as u64);
    assert_eq!(preview.stats.values.as_ref().map(Vec::len), Some(1));

    // Facilities: FlatGeobuf with every attribute.
    let Summary::Vector(vector) = &all[2].summary else {
        panic!("vector summary");
    };
    assert_eq!(vector.features, 1);
    assert_eq!(vector.fields, ["hf_id", "name"]);

    drop(project);
    let again = Project::open(&dir.path().join("p")).expect("open");
    assert_eq!(
        again
            .datasets(Some(DataClass::Facility))
            .expect("list")
            .len(),
        1
    );
}

#[test]
fn tags_name_datasets_uniquely_per_class() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let err = project
        .import(
            &dir.path().join("lc.tif"),
            DataClass::LandCoverMerged,
            &tags(&["base"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect_err("duplicate");
    assert!(err.to_string().contains("already"), "{err}");
    // Nothing is left of the refused import.
    assert_eq!(project.datasets(None).expect("list").len(), 3);
    assert!(!dir.path().join("p/datasets/4.tif").exists());
    // Same tags, other class: allowed.
    project
        .import(
            &dir.path().join("lc.tif"),
            DataClass::LandCover,
            &tags(&["base"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("other class");
    let retagged = project.retag(2, &tags(&["base 2024"])).expect("retag");
    assert_eq!(retagged.tags, ["base", "2024"]);
}

/// A shapefile declares "Polygon" for polygons and multipolygons alike
/// (AccessMod 5 writes such layers): the copy is multi-part, every feature
/// kept. FlatGeobuf refused the multipolygon before.
#[test]
fn shapefiles_mixing_single_and_multi_parts_import() {
    use gdal::vector::{Feature, Geometry, LayerAccess, LayerOptions, OGRwkbGeometryType};
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let path = dir.path().join("zones.shp");
    {
        let driver = gdal::DriverManager::get_driver_by_name("ESRI Shapefile").expect("driver");
        let mut ds = driver.create_vector_only(&path).expect("create");
        let srs = gdal::spatial_ref::SpatialRef::from_epsg(EPSG).expect("srs");
        let layer = ds
            .create_layer(LayerOptions {
                name: "zones",
                srs: Some(&srs),
                ty: OGRwkbGeometryType::wkbPolygon,
                options: None,
            })
            .expect("layer");
        let single = fixtures_polygon(0.0, 0.0, 2.0, 2.0);
        let multi = format!(
            "MULTIPOLYGON ({}, {})",
            fixtures_polygon(3.0, 0.0, 4.0, 1.0).trim_start_matches("POLYGON "),
            fixtures_polygon(4.0, 2.0, 5.0, 3.0).trim_start_matches("POLYGON ")
        );
        for wkt in [single, multi] {
            let mut feature = Feature::new(layer.defn()).expect("feature");
            feature
                .set_geometry(Geometry::from_wkt(&wkt).expect("wkt"))
                .expect("geometry");
            feature.create(&layer).expect("create feature");
        }
    }
    let record = project
        .import(
            &path,
            DataClass::Zone,
            &tags(&["mixed"]),
            &ImportOptions::default(),
            &settings(64 << 20, 1),
            &|| false,
        )
        .expect("import");
    let Summary::Vector(summary) = record.summary else {
        panic!("vector");
    };
    assert_eq!(summary.features, 2);
    assert_eq!(summary.geometry, "Multi Polygon");
}

fn fixtures_polygon(c0: f64, r0: f64, c1: f64, r1: f64) -> String {
    accessmod_io::fixtures::cell_polygon(c0, r0, c1, r1)
}

#[test]
fn imports_check_lattice_and_geometry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    // Half a cell off the lattice.
    write_raster::<f32>(
        &dir.path().join("off.tif"),
        COLS,
        ROWS,
        geo_transform(X0 + 50.0, Y0),
        vec![1.0; COLS * ROWS],
        None,
    );
    let err = project
        .import(
            &dir.path().join("off.tif"),
            DataClass::Population,
            &tags(&["off"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect_err("misaligned");
    assert_eq!(err.kind(), accessmod_io::error::ErrorKind::Validation);
    // Facilities must be points.
    write_features(
        &dir.path().join("zones.gpkg"),
        "zones",
        EPSG,
        &[("id", OGRFieldType::OFTInteger)],
        &[(
            &format!(
                "POLYGON (({X0} {Y0}, {} {Y0}, {} {}, {X0} {Y0}))",
                X0 + 100.0,
                X0 + 100.0,
                Y0 - 100.0
            ),
            vec![Some(FieldValue::IntegerValue(1))],
        )],
    );
    let err = project
        .import(
            &dir.path().join("zones.gpkg"),
            DataClass::Facility,
            &tags(&["wrong"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect_err("polygons as facilities");
    assert!(err.to_string().contains("polygon"), "{err}");
    project
        .import(
            &dir.path().join("zones.gpkg"),
            DataClass::Zone,
            &tags(&["districts"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("zones");
}

#[test]
fn smaller_raster_is_padded_with_nodata() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    // Two columns, starting at column 2, without NoData of its own.
    write_raster::<f32>(
        &dir.path().join("pop.tif"),
        2,
        ROWS,
        geo_transform(X0 + 200.0, Y0),
        vec![5.0; 2 * ROWS],
        None,
    );
    let record = project
        .import(
            &dir.path().join("pop.tif"),
            DataClass::Population,
            &tags(&["pop"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("population");
    let values: Vec<f32> = read(&project.root().join(&record.file));
    let present: Vec<bool> = values.iter().map(|v| !v.is_nan()).collect();
    for row in 0..ROWS {
        for col in 0..COLS {
            assert_eq!(
                present[row * COLS + col],
                (2..4).contains(&col),
                "({row}, {col})"
            );
        }
    }
}

#[test]
fn run_registers_outputs_as_datasets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let record = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &[],
            false,
            &settings,
            Monitor::QUIET,
        )
        .expect("run");
    assert_eq!(record.status, RunStatus::Done);
    assert_eq!(record.output_dir, "runs/1");
    assert_eq!(record.tags, ["travel", "time", "run1"]);
    assert!(
        record
            .outcome
            .as_deref()
            .unwrap_or("")
            .contains("cells reached")
    );
    assert_eq!(record.job["output_dir"], "runs/1");
    // The datasets it read: DEM, land cover, facilities.
    assert_eq!(record.inputs, [1, 2, 3]);

    let outputs: Vec<(DataClass, String)> = record
        .datasets
        .iter()
        .map(|&id| {
            let d = project.dataset(id).expect("dataset");
            (d.class, d.file)
        })
        .collect();
    assert_eq!(
        outputs,
        [
            (
                DataClass::TravelTime,
                "runs/1/travel_time_seconds.tif".to_owned()
            ),
            (
                DataClass::CostAllocation,
                "runs/1/nearest_facility.tif".to_owned()
            ),
        ]
    );
    // The output is the CLI's: same file a plain run writes.
    assert!(project.root().join("runs/1/run.json").is_file());
    assert!(
        project
            .root()
            .join("runs/1/travel_time_seconds.tif.ovr")
            .exists()
            || COLS < 256
    );

    // Same tags again: refused before running, recorded as failed.
    let err = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &tags(&["travel time run1"]),
            false,
            &settings,
            Monitor::QUIET,
        )
        .expect_err("tags taken");
    assert!(err.to_string().contains("already"), "{err}");
    let runs = project.runs().expect("runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].status, RunStatus::Failed);
    assert!(runs[1].inputs.is_empty());
    assert!(!project.root().join("runs/2").exists());

    // With overwrite: nothing replaced while the run has not succeeded…
    let queued = project
        .start_named(
            "travel-time",
            &travel_time_job(),
            &tags(&["travel time run1"]),
            true,
        )
        .expect("queued");
    assert!(project.run_record(queued).expect("run").overwrite);
    assert!(project.cancel_queued(queued).expect("cancel"));
    assert!(project.dataset(record.datasets[0]).is_ok());
    // …then the datasets of its classes and tags are replaced, files included.
    let again = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &tags(&["travel time run1"]),
            true,
            &settings,
            Monitor::QUIET,
        )
        .expect("overwrite");
    assert_eq!(again.status, RunStatus::Done);
    assert_eq!(again.tags, record.tags);
    // Its job is named by its tags and its number: the first keeps the tags.
    assert_eq!(
        project.run_record(record.id).expect("run").job_tags,
        record.tags
    );
    let mut numbered = record.tags.clone();
    numbered.push(again.id.to_string());
    assert_eq!(again.job_tags, numbered);
    assert!(again.outcome.as_deref().unwrap_or("").contains("replaced"));
    for (old, new) in record.datasets.iter().zip(&again.datasets) {
        assert!(project.dataset(*old).is_err());
        let new = project.dataset(*new).expect("new");
        assert!(new.file.starts_with(&again.output_dir));
    }
    assert!(
        !project
            .root()
            .join("runs/1/travel_time_seconds.tif")
            .exists()
    );
    assert!(project.run_record(1).expect("run").datasets.is_empty());

    // A removed input leaves the run's inputs.
    project.remove_dataset(3).expect("remove facilities");
    assert_eq!(project.run_record(1).expect("run").inputs, [1, 2]);
}

/// Runs queued or running when the project is exported (the export's own
/// run among them) are left out of the file: imported, they would only
/// read "interrupted".
#[test]
fn project_files_leave_unfinished_runs_out() {
    use accessmod_io::archive::Limits;

    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let run = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &[],
            false,
            &settings(64 << 20, 1),
            Monitor::QUIET,
        )
        .expect("run");
    project
        .start_named("travel-time", &travel_time_job(), &[], false)
        .expect("queue");
    let file = project
        .export_package(&dir.path().join("exports"), true, Monitor::QUIET)
        .expect("export");
    let copy = Project::import_package(
        &file,
        &dir.path().join("copy"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect("import");
    let runs = copy.runs().expect("runs");
    assert_eq!(
        runs.iter().map(|r| r.id).collect::<Vec<_>>(),
        [run.id],
        "{runs:?}"
    );
}

/// A project restored at the path of one removed is another: its file keeps
/// the creation time, not the instance, so a removal asked for the first
/// spares it.
#[test]
fn a_restored_project_is_another_instance() {
    use accessmod_io::archive::Limits;

    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let root = project.root().to_path_buf();
    let first = project.instance().expect("instance");
    assert_eq!(project.instance().expect("instance"), first);
    let file = project
        .export_package(&dir.path().join("exports"), false, Monitor::QUIET)
        .expect("export");
    project.remove().expect("remove");
    let again =
        Project::import_package(&file, &root, Limits::default(), Monitor::QUIET).expect("restore");
    let second = again.instance().expect("instance");
    assert_ne!(second, first);
    drop(again);
    let err = Project::open(&root)
        .expect("open")
        .remove_if(&first)
        .expect_err("another instance");
    assert!(err.to_string().contains("made again"), "{err}");
    assert!(root.exists());
    Project::open(&root)
        .expect("open")
        .remove_if(&second)
        .expect("remove");
    assert!(!root.exists());
}

#[test]
fn project_files_round_trip_with_or_without_results() {
    use accessmod_io::archive::{Entry, Limits, Source};

    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let run = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &[],
            false,
            &settings,
            Monitor::QUIET,
        )
        .expect("run");
    let exports = dir.path().join("exports");

    // With results: the same catalogue, runs and files.
    let file = project
        .export_package(&exports, true, Monitor::QUIET)
        .expect("export");
    assert!(file.extension().is_some_and(|e| e == "am6p"));
    let copy = Project::import_package(
        &file,
        &dir.path().join("copy"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect("import");
    assert_eq!(copy.info().expect("info").name, "Demo");
    assert_eq!(
        copy.datasets(None).expect("list"),
        project.datasets(None).expect("list")
    );
    assert_eq!(copy.runs().expect("runs"), project.runs().expect("runs"));
    for d in copy.datasets(None).expect("list") {
        assert_eq!(
            std::fs::read(copy.root().join(&d.file)).expect("copied"),
            std::fs::read(project.root().join(&d.file)).expect("original"),
        );
    }
    // Never into an existing directory.
    assert!(
        Project::import_package(
            &file,
            &dir.path().join("copy"),
            Limits::default(),
            Monitor::QUIET
        )
        .is_err()
    );

    // Without results: imported datasets only; runs keep their jobs.
    let light = project
        .export_package(&exports, false, Monitor::QUIET)
        .expect("export");
    assert_ne!(light, file);
    let copy = Project::import_package(
        &light,
        &dir.path().join("light"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect("import");
    let kept: Vec<i64> = copy
        .datasets(None)
        .expect("list")
        .iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(kept, [1, 2, 3]);
    let runs = copy.runs().expect("runs");
    assert_eq!(runs.len(), 1);
    assert!(runs[0].datasets.is_empty());
    assert_eq!(runs[0].job, run.job);
    assert!(!copy.root().join("runs/1/travel_time_seconds.tif").exists());

    // A data archive is not a project file; a database with code is refused.
    let data = project
        .create_archive("", &[1], &[], Monitor::QUIET)
        .expect("archive");
    let err = Project::import_package(
        &project.archive_path(&data.name).expect("path"),
        &dir.path().join("bad"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect_err("data archive");
    assert!(
        err.to_string().contains("not an AccessMod project file"),
        "{err}"
    );
    assert!(!dir.path().join("bad").exists());
    let db = dir.path().join("trigger.sqlite");
    std::fs::copy(copy.root().join("project.sqlite"), &db).expect("copy db");
    rusqlite::Connection::open(&db)
        .and_then(|c| {
            c.execute_batch(
                "CREATE TRIGGER t AFTER INSERT ON runs BEGIN DELETE FROM datasets; END;",
            )
        })
        .expect("trigger");
    let (hostile, _) = accessmod_io::archive::write(
        dir.path(),
        &mut std::iter::once("hostile.am6p".to_owned()),
        &[Entry {
            name: "project.sqlite".into(),
            source: Source::File(db),
        }],
        "am6p",
        serde_json::Value::Null,
        Monitor::QUIET,
    )
    .expect("hostile");
    let err = Project::import_package(
        &hostile,
        &dir.path().join("bad"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect_err("trigger");
    assert!(err.to_string().contains("schema differs"), "{err}");
    assert!(!dir.path().join("bad").exists());

    // The files of the light copy (closed: its database checkpointed),
    // packed again with a change.
    drop(copy);
    let repack = |extra: &[(&str, &[u8])], replace: Option<(&str, &[u8])>| {
        let root = dir.path().join("light");
        let mut entries = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(d) = pending.pop() {
            for e in std::fs::read_dir(&d).expect("dir").flatten() {
                let rel = e
                    .path()
                    .strip_prefix(&root)
                    .expect("rel")
                    .to_string_lossy()
                    .replace('\\', "/");
                if e.file_type().expect("type").is_dir() {
                    pending.push(e.path());
                } else if !rel.starts_with('.') && !rel.ends_with("-wal") && !rel.ends_with("-shm")
                {
                    let source = match replace {
                        Some((name, bytes)) if name == rel => Source::Bytes(bytes.to_vec()),
                        _ => Source::File(e.path()),
                    };
                    entries.push(Entry { name: rel, source });
                }
            }
        }
        for (name, bytes) in extra {
            entries.push(Entry {
                name: (*name).to_owned(),
                source: Source::Bytes(bytes.to_vec()),
            });
        }
        let _ = std::fs::remove_file(dir.path().join("repacked.am6p"));
        accessmod_io::archive::write(
            dir.path(),
            &mut std::iter::once("repacked.am6p".to_owned()),
            &entries,
            "am6p",
            serde_json::Value::Null,
            Monitor::QUIET,
        )
        .expect("repack")
        .0
    };
    // A sidecar GDAL would read beside a dataset.
    let sidecar = repack(&[("datasets/1.tif.aux.xml", b"<PAMDataset/>")], None);
    let err = Project::import_package(
        &sidecar,
        &dir.path().join("bad"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect_err("sidecar");
    assert!(err.to_string().contains("beside a dataset"), "{err}");
    // A dataset that is a VRT reading another file.
    let vrt = format!(
        "<VRTDataset rasterXSize=\"6\" rasterYSize=\"4\"><VRTRasterBand dataType=\"Float32\" band=\"1\"><SimpleSource><SourceFilename>{}</SourceFilename><SourceBand>1</SourceBand></SimpleSource></VRTRasterBand></VRTDataset>",
        project.root().join("datasets/1.tif").display()
    );
    let hidden = repack(&[], Some(("datasets/1.tif", vrt.as_bytes())));
    let err = Project::import_package(
        &hidden,
        &dir.path().join("bad"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect_err("vrt");
    assert!(err.to_string().contains("not a supported file"), "{err}");
    assert!(!dir.path().join("bad").exists());
    // The light copy itself, repacked unchanged, imports.
    let same = repack(&[], None);
    Project::import_package(
        &same,
        &dir.path().join("again"),
        Limits::default(),
        Monitor::QUIET,
    )
    .expect("repacked");
    // No staging directory left behind.
    assert!(
        std::fs::read_dir(dir.path())
            .expect("dir")
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().starts_with(".import-"))
    );
}

#[test]
fn archived_runs_replay_to_identical_outputs() {
    use accessmod_io::analysis::Analysis;
    use accessmod_io::archive::{Limits, extract};
    use accessmod_io::job::JobFile;

    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let run = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &[],
            false,
            &settings,
            Monitor::QUIET,
        )
        .expect("run");
    // The run filter of Data: its inputs and outputs, and its job.
    let datasets: Vec<i64> = run.inputs.iter().chain(&run.datasets).copied().collect();
    let info = project
        .create_archive("e2e", &datasets, &[run.id], Monitor::QUIET)
        .expect("archive");
    assert!(info.name.starts_with("e2e_") && info.name.ends_with(".zip"));
    assert_eq!(
        project.archives().expect("list"),
        std::slice::from_ref(&info)
    );

    let out = dir.path().join("out");
    let manifest = extract(
        &project.archive_path(&info.name).expect("path"),
        &out,
        Limits::default(),
        Monitor::QUIET,
    )
    .expect("extract");
    assert_eq!(manifest.format, "accessmod-data");
    assert_eq!(manifest.content["runs"][0]["replayable"], true);
    // No overview: they are rebuilt.
    assert!(manifest.files.iter().all(|f| !f.path.ends_with(".ovr")));
    assert!(out.join("runs/1/run.json").is_file());

    // `accessmod travel-time run --job job_run1.json` in the extracted folder.
    let job = <TravelTime as Analysis>::Job::from_file(&out.join("job_run1.json")).expect("job");
    accessmod_io::fixtures::run_job::<TravelTime>(job, &settings).expect("replay");
    for file in ["travel_time_seconds.tif", "nearest_facility.tif"] {
        assert_eq!(
            std::fs::read(out.join("replay_run1").join(file)).expect("replayed"),
            std::fs::read(project.root().join("runs/1").join(file)).expect("original"),
            "{file}"
        );
    }

    // Without its inputs, the run is not replayable; archives are removed by name.
    let partial = project
        .create_archive("", &run.datasets, &[run.id], Monitor::QUIET)
        .expect("archive");
    assert!(partial.name.starts_with("am6_export_"));
    let manifest = accessmod_io::archive::manifest(
        &project.archive_path(&partial.name).expect("path"),
        Limits::default(),
    )
    .expect("manifest");
    assert_eq!(manifest.content["runs"][0]["replayable"], false);
    project.remove_archive(&partial.name).expect("remove");
    assert_eq!(project.archives().expect("list").len(), 1);
    for bad in ["../project.sqlite", "x.zip", ".hidden.zip", "a/b.zip"] {
        assert!(project.archive_path(bad).is_err(), "{bad}");
    }
    assert!(
        project
            .create_archive("", &[], &[], Monitor::QUIET)
            .is_err()
    );
    assert!(
        project
            .create_archive("bad name!", &[1], &[], Monitor::QUIET)
            .is_err()
    );

    // An input removed since the run: every remaining file archived, still
    // not replayable (the job names the removed file).
    project.remove_dataset(2).expect("remove land cover");
    let mut rest: Vec<i64> = run.inputs.iter().chain(&run.datasets).copied().collect();
    rest.retain(|&id| id != 2);
    let gone = project
        .create_archive("gone", &rest, &[run.id], Monitor::QUIET)
        .expect("archive");
    let manifest = accessmod_io::archive::manifest(
        &project.archive_path(&gone.name).expect("path"),
        Limits::default(),
    )
    .expect("manifest");
    assert_eq!(manifest.content["runs"][0]["replayable"], false);

    // Files that are links out of the project are never archived nor served.
    #[cfg(unix)]
    {
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "secret").expect("secret");
        let facilities = project.root().join(project.dataset(3).expect("3").file);
        std::fs::remove_file(&facilities).expect("rm");
        std::os::unix::fs::symlink(&secret, &facilities).expect("link");
        let err = project
            .create_archive("", &[3], &[], Monitor::QUIET)
            .expect_err("link");
        assert!(err.to_string().contains("leaves the project"), "{err}");
        std::os::unix::fs::symlink(&secret, project.root().join("archives/evil.zip"))
            .expect("link");
        assert!(project.archive_path("evil.zip").is_err());
        assert!(
            project
                .archives()
                .expect("list")
                .iter()
                .all(|a| a.name != "evil.zip")
        );
    }
}

#[test]
fn overwrite_spares_datasets_that_other_runs_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let raw = project
        .import(
            &dir.path().join("lc.tif"),
            DataClass::LandCover,
            &tags(&["raw"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("land cover");
    // A merge tagged `base` replaces the merged land cover [base] (dataset 2)…
    let merge = json!({
        "schema_version": 1,
        "dem": "datasets/1.tif",
        "stack": [{"type": "land_cover", "path": raw.file}],
    });
    // …not while a queued run reads it.
    let queued = project
        .start_run::<TravelTime>(&travel_time_job(), &[], false)
        .expect("queued");
    let err = project
        .run_named(
            "merge-land-cover",
            &merge,
            &tags(&["base"]),
            true,
            &settings,
            Monitor::QUIET,
        )
        .expect_err("read by a queued run");
    assert!(err.to_string().contains("is read by run"), "{err}");
    assert!(project.dataset(2).is_ok());
    // Once that run ends, it does.
    assert!(project.cancel_queued(queued).expect("cancel"));
    let done = project
        .run_named(
            "merge-land-cover",
            &merge,
            &tags(&["base"]),
            true,
            &settings,
            Monitor::QUIET,
        )
        .expect("overwrite");
    assert!(project.dataset(2).is_err());
    assert!(!project.root().join("datasets/2.tif").exists());
    let merged = project
        .datasets(Some(DataClass::LandCoverMerged))
        .expect("list");
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].tags, ["base"]);
    assert!(done.datasets.contains(&merged[0].id));
}

#[test]
fn failed_runs_record_their_issues() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let mut job = travel_time_job();
    job["facilities"]["id_field"] = json!("missing_field");
    let err = project
        .run_named("travel-time", &job, &[], false, &settings, Monitor::QUIET)
        .expect_err("invalid");
    assert_eq!(err.kind(), accessmod_io::error::ErrorKind::Validation);
    let run = project.run_record(1).expect("run");
    assert_eq!(run.status, RunStatus::Failed);
    let error = run.error.expect("error");
    assert_eq!(error.kind, "validation");
    assert!(
        error.issues.iter().any(|i| i.contains("missing_field")),
        "{error:?}"
    );
    assert!(run.datasets.is_empty());
}

#[test]
fn jobs_cannot_leave_the_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let mut job = travel_time_job();
    job["land_cover"] = json!("../lc.tif");
    let err = project.check_job::<TravelTime>(&job).expect_err("outside");
    assert!(err.to_string().contains("leaves the project"), "{err}");
    job["land_cover"] = json!(dir.path().join("lc.tif"));
    assert!(
        project.check_job::<TravelTime>(&job).is_err(),
        "absolute path"
    );
    // Nothing was recorded.
    assert!(project.runs().expect("runs").is_empty());
}

#[test]
fn saved_jobs_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let draft = json!({"dem": "datasets/1.tif"});
    let saved = project
        .save_job(None, "travel-time", "draft", &draft)
        .expect("save");
    assert_eq!(saved.document, draft);
    let updated = project
        .save_job(
            Some((saved.id, saved.revision)),
            "travel-time",
            "renamed",
            &travel_time_job(),
        )
        .expect("update");
    assert_eq!(updated.name, "renamed");
    assert_eq!(project.jobs().expect("jobs").len(), 1);
    assert!(
        project
            .save_job(None, "no-such-analysis", "x", &draft)
            .is_err()
    );
    project.remove_job(saved.id).expect("remove");
    assert!(project.jobs().expect("jobs").is_empty());
}

#[test]
fn recover_cleans_interrupted_work() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    // One run queued, one running when the process stopped.
    project
        .start_run::<TravelTime>(&travel_time_job(), &[], false)
        .expect("queue");
    project
        .start_run::<TravelTime>(&travel_time_job(), &[], false)
        .expect("queue");
    assert_eq!(
        project.run_record(1).expect("run").status,
        RunStatus::Queued
    );
    project.recover().expect("recover");
    assert_eq!(
        project.run_record(1).expect("run").status,
        RunStatus::Interrupted
    );
}

#[test]
fn dem_cannot_be_removed_and_others_can() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    assert!(project.remove_dataset(1).is_err());
    project.remove_dataset(2).expect("remove land cover");
    assert!(!project.root().join("datasets/2.tif").exists());
    assert!(project.dataset(2).is_err());
}

#[test]
fn removal_waits_for_activity_and_queued_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("p");
    let project = project_with_inputs(dir.path());
    // An import or a run holds the project.
    let activity = project.activity().expect("activity");
    let project = Project::open(&root).expect("open");
    assert!(project.remove().is_err());
    drop(activity);
    // A queued run too.
    let project = Project::open(&root).expect("open");
    let run = project
        .start_run::<TravelTime>(&travel_time_job(), &[], false)
        .expect("queue");
    let project = Project::open(&root).expect("open");
    assert!(project.remove().is_err());
    let project = Project::open(&root).expect("open");
    assert!(project.cancel_queued(run).expect("cancel"));
    // Siblings are never touched, whatever their name.
    let sibling = dir.path().join("p.deleting");
    std::fs::create_dir(&sibling).expect("mkdir");
    std::fs::write(sibling.join("keep"), "x").expect("write");
    // A handle opened before the removal can no longer import or queue.
    let stale = Project::open(&root).expect("open");
    project.remove().expect("remove");
    assert!(!root.exists());
    assert!(sibling.join("keep").exists());
    assert!(stale.activity().is_err());
    assert!(
        stale
            .start_run::<TravelTime>(&travel_time_job(), &[], false)
            .is_err()
    );
    accessmod_io::project::purge_removed(dir.path());
    assert!(std::fs::read_dir(dir.path()).expect("list").all(|e| {
        let name = e.expect("entry").file_name();
        !name.to_string_lossy().starts_with("p.removed-")
    }));
}

#[test]
fn creation_needs_an_empty_directory_and_a_valid_dem() {
    let dir = tempfile::tempdir().expect("tempdir");
    sources(dir.path());
    let settings = settings(64 << 20, 1);
    std::fs::create_dir(dir.path().join("busy")).expect("mkdir");
    std::fs::write(dir.path().join("busy/x"), "x").expect("write");
    assert!(
        Project::create(
            &dir.path().join("busy"),
            "A",
            "",
            &dir.path().join("dem.tif"),
            &settings,
            &|| false
        )
        .is_err()
    );
    // Land cover in a geographic CRS cannot define a grid... a missing file neither.
    let err = Project::create(
        &dir.path().join("new"),
        "A",
        "",
        &dir.path().join("none.tif"),
        &settings,
        &|| false,
    )
    .expect_err("no dem");
    assert!(!dir.path().join("new").exists(), "{err}");
}

#[test]
fn queued_runs_execute_later_or_cancel() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let first = project
        .start_named("travel-time", &travel_time_job(), &[], false)
        .expect("queue");
    let second = project
        .start_named("travel-time", &travel_time_job(), &[], false)
        .expect("queue");
    assert!(project.cancel_queued(second).expect("cancel"));
    assert!(!project.cancel_queued(second).expect("already cancelled"));
    assert_eq!(
        project.run_record(second).expect("run").status,
        RunStatus::Cancelled
    );
    let done = project
        .execute_queued(first, &settings, Monitor::QUIET)
        .expect("execute");
    assert_eq!(done.status, RunStatus::Done);
    assert!(
        project
            .execute_queued(second, &settings, Monitor::QUIET)
            .is_err()
    );
    // Cancelled while waiting: recorded as cancelled, nothing written.
    let third = project
        .start_named("travel-time", &travel_time_job(), &[], false)
        .expect("queue");
    let err = project
        .execute_queued(third, &settings, Monitor::new(&|| true, &|_| {}))
        .expect_err("cancelled");
    assert_eq!(err.kind(), accessmod_io::error::ErrorKind::Cancelled);
    assert_eq!(
        project.run_record(third).expect("run").status,
        RunStatus::Cancelled
    );
    assert!(!project.root().join(format!("runs/{third}")).exists());
}

#[test]
fn jobs_name_catalogue_datasets_of_the_right_class() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    // A file of the project that is not a dataset.
    let mut job = travel_time_job();
    job["land_cover"] = json!("project.sqlite");
    let err = project
        .check_job::<TravelTime>(&job)
        .expect_err("not a dataset");
    assert!(err.to_string().contains("not a dataset"), "{err}");
    // A dataset of a class the field does not take: the DEM as land cover.
    job["land_cover"] = json!("datasets/1.tif");
    let err = project
        .check_job::<TravelTime>(&job)
        .expect_err("wrong class");
    assert!(err.to_string().contains("rDem dataset"), "{err}");
    // `./` prefixes name the same dataset.
    job["land_cover"] = json!("./datasets/2.tif");
    project.check_job::<TravelTime>(&job).expect("same dataset");
}

#[test]
fn outputs_of_analyses_are_not_imported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let err = project
        .import(
            &dir.path().join("lc.tif"),
            DataClass::TravelTime,
            &tags(&["x"]),
            &ImportOptions::default(),
            &settings(64 << 20, 1),
            &|| false,
        )
        .expect_err("output class");
    assert!(err.to_string().contains("written by analyses"), "{err}");
}

#[test]
fn partial_rasters_get_a_nodata_value_they_do_not_hold() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    // Byte classes using 255: NoData must be another value.
    write_raster::<u8>(
        &dir.path().join("excl.tif"),
        2,
        ROWS,
        geo_transform(X0, Y0),
        vec![255, 1, 255, 1, 255, 1, 255, 1],
        None,
    );
    let record = project
        .import(
            &dir.path().join("excl.tif"),
            DataClass::Exclusion,
            &tags(&["zones"]),
            &ImportOptions::default(),
            &settings(64 << 20, 1),
            &|| false,
        )
        .expect("import");
    let (values, nodata) =
        accessmod_io::fixtures::read_with_nodata::<u8>(&project.root().join(&record.file));
    assert_eq!(nodata, Some(0.0), "255 is held, so 0 (u8::MIN)");
    assert_eq!(values[0], 255);
    assert_eq!(values[COLS - 1], 0);
}

#[test]
fn saved_jobs_detect_concurrent_edits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let saved = project
        .save_job(None, "travel-time", "draft", &json!({}))
        .expect("save");
    project
        .save_job(
            Some((saved.id, saved.revision)),
            "travel-time",
            "first",
            &json!({}),
        )
        .expect("first edit");
    let err = project
        .save_job(
            Some((saved.id, saved.revision)),
            "travel-time",
            "second",
            &json!({}),
        )
        .expect_err("stale revision");
    assert!(err.to_string().contains("changed since revision"), "{err}");
}

#[test]
fn recovery_waits_for_work_in_progress() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let hold = project.activity().expect("hold");
    assert!(!project.recover().expect("recover"), "busy: nothing done");
    drop(hold);
    assert!(project.recover().expect("recover"));
}

#[test]
fn tables_with_quoted_fields_are_summarised() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    std::fs::write(
        dir.path().join("scenario.csv"),
        "class,label,speed\n1,\"forest, dense\",2\n2,\"road\nmain\",60\n",
    )
    .expect("write");
    let record = project
        .import(
            &dir.path().join("scenario.csv"),
            DataClass::Scenario,
            &tags(&["dry season"]),
            &ImportOptions::default(),
            &settings(64 << 20, 1),
            &|| false,
        )
        .expect("table");
    let Summary::Table(table) = record.summary else {
        panic!("table summary");
    };
    assert_eq!(table.columns, ["class", "label", "speed"]);
    assert_eq!(table.rows, 2);
}

#[test]
fn erdas_imagine_rasters_are_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let tif = gdal::Dataset::open(dir.path().join("lc.tif")).expect("open");
    let img = dir.path().join("lc.img");
    let hfa = gdal::DriverManager::get_driver_by_name("HFA").expect("HFA driver");
    tif.create_copy(&hfa, &img, &gdal::cpl::CslStringList::new())
        .expect("write HFA");
    drop(tif);
    accessmod_io::project::check_self_contained(&img).expect("self-contained");
    let record = project
        .import(
            &img,
            DataClass::LandCover,
            &tags(&["erdas"]),
            &ImportOptions::default(),
            &settings(64 << 20, 1),
            &|| false,
        )
        .expect("import");
    assert!(matches!(record.summary, Summary::Raster(_)));
}

#[test]
fn xlsx_sheets_are_imported_as_csv() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let path = dir.path().join("tables.xlsx");
    // Written without GDAL: bundled GDAL builds have no XLSX driver.
    let mut book = rust_xlsxwriter::Workbook::new();
    for (name, rows) in [("classes", 2u32), ("other", 1)] {
        let sheet = book.add_worksheet().set_name(name).expect("sheet");
        sheet.write_string(0, 0, "class").expect("header");
        sheet.write_string(0, 1, "label").expect("header");
        for i in 0..rows {
            sheet
                .write_number(i + 1, 0, f64::from(i + 1))
                .expect("class");
            sheet
                .write_string(i + 1, 1, format!("forest, {i}"))
                .expect("label");
        }
    }
    book.save(&path).expect("save");
    accessmod_io::project::check_self_contained(&path).expect("self-contained");
    let import = |layer: Option<&str>, tag: &str| {
        project.import(
            &path,
            DataClass::LandCoverTable,
            &tags(&[tag]),
            &ImportOptions {
                layer: layer.map(str::to_owned),
            },
            &settings(64 << 20, 1),
            &|| false,
        )
    };
    let record = import(None, "first").expect("first sheet");
    let Summary::Table(table) = record.summary else {
        panic!("table summary");
    };
    assert_eq!(table.columns, ["class", "label"]);
    assert_eq!(table.rows, 2);
    let csv = std::fs::read_to_string(project.root().join(&record.file)).expect("csv");
    assert_eq!(csv, "class,label\n1,\"forest, 0\"\n2,\"forest, 1\"\n");
    let record = import(Some("other"), "second").expect("named sheet");
    let Summary::Table(table) = record.summary else {
        panic!("table summary");
    };
    assert_eq!(table.rows, 1);
    assert!(import(Some("missing"), "third").is_err());
}

#[test]
fn demo_project_holds_every_sample_dataset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("demo");
    let project =
        accessmod_io::demo::create(&root, &settings(64 << 20, 1), &|| false).expect("demo");
    let info = project.info().expect("info");
    assert!(info.demo);
    assert_eq!(info.name, accessmod_io::demo::NAME);
    assert_eq!((info.grid.cols, info.grid.rows), (227, 196));
    let datasets = project.datasets(None).expect("datasets");
    // The DEM, then every sample dataset.
    assert_eq!(datasets.len(), accessmod_io::demo::DATASETS.len() + 1);
    assert!(
        datasets
            .iter()
            .all(|d| d.tags.first().map(String::as_str) == Some(accessmod_io::demo::TAG))
    );
    for (_, class, tags) in accessmod_io::demo::DATASETS {
        assert!(
            datasets
                .iter()
                .any(|d| d.class == *class && d.tags == *tags),
            "{class:?} {tags:?}"
        );
    }
    let facilities = project
        .datasets(Some(DataClass::Facility))
        .expect("facilities");
    let Summary::Vector(vector) = &facilities[0].summary else {
        panic!("vector summary");
    };
    assert_eq!(vector.features, 10);
    let capacity = project
        .datasets(Some(DataClass::Capacity))
        .expect("capacity");
    let Summary::Table(table) = &capacity[0].summary else {
        panic!("table summary");
    };
    assert_eq!(table.columns, ["min", "max", "label", "capacity"]);
    assert_eq!(table.rows, 7);
    // Only the project is left next to it.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(entries, ["demo"]);
    // A project that is not the demo says so.
    assert!(!project_with_inputs(dir.path()).info().expect("info").demo);
}

#[test]
fn uploads_read_by_another_driver_than_their_name_are_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    sources(dir.path());
    let outside = dir.path().join("lc.tif");
    let upload = dir.path().join("upload");
    std::fs::create_dir(&upload).expect("dir");
    let vrt = format!(
        "<VRTDataset rasterXSize=\"{COLS}\" rasterYSize=\"{ROWS}\">\
         <VRTRasterBand dataType=\"Byte\" band=\"1\"><SimpleSource>\
         <SourceFilename relativeToVRT=\"0\">{}</SourceFilename>\
         <SourceBand>1</SourceBand></SimpleSource></VRTRasterBand></VRTDataset>",
        outside.display()
    );
    let ogr_vrt = format!(
        "<OGRVRTDataSource><OGRVRTLayer name=\"hf\"><SrcDataSource>{}</SrcDataSource>\
         </OGRVRTLayer></OGRVRTDataSource>",
        dir.path().join("hf.gpkg").display()
    );
    // The danger is GDAL reading a VRT, hence the file it names. Bundled
    // builds cannot open a VRT raster at all: such a file is then plain
    // text, which the CSV driver may accept.
    let real = dir.path().join("real.vrt");
    std::fs::write(&real, &vrt).expect("write");
    let reads_vrt = gdal::Dataset::open(&real).is_ok();
    for (name, content) in [("dem.csv", &vrt), ("dem.tif", &vrt), ("hf.csv", &ogr_vrt)] {
        let path = upload.join(name);
        std::fs::write(&path, content).expect("write");
        assert!(
            accessmod_io::project::check_self_contained(&path).is_err() || !reads_vrt,
            "{name} is refused"
        );
        std::fs::remove_file(&path).expect("remove");
    }
    let table = upload.join("scenario.csv");
    std::fs::write(&table, "class,label,speed\n1,\"forest, dense\",2\n").expect("write");
    accessmod_io::project::check_self_contained(&table).expect("a CSV table");
}

#[test]
fn relocation_writes_a_new_facilities_dataset() {
    use accessmod_io::vector::PointMove;
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let facilities = project.dataset(3).expect("facilities");
    let source = project.resolve(Path::new(&facilities.file)).expect("path");
    let before = accessmod_io::tiles::geojson(&source, 10, None).expect("geojson");
    let feature = &before["features"][0];
    let fid = feature["id"].as_u64().expect("fid");
    let [lon, lat] = [0, 1].map(|i| {
        feature["geometry"]["coordinates"][i]
            .as_f64()
            .expect("coordinate")
    });

    let moved = PointMove {
        fid,
        lon: lon + 0.0005,
        lat: lat - 0.0005,
    };
    let record = project
        .relocate(3, &[moved], &tags(&["relocated"]), &settings, &|| false)
        .expect("relocate");
    assert_eq!(record.class, DataClass::Facility);
    assert_eq!(record.tags, tags(&["relocated"]));
    let after = accessmod_io::tiles::geojson(
        &project.resolve(Path::new(&record.file)).expect("path"),
        10,
        None,
    )
    .expect("geojson");
    let feature = &after["features"][0];
    assert_eq!(
        feature["properties"],
        json!({"hf_id": 7, "name": "Central"})
    );
    for (i, expected) in [moved.lon, moved.lat].into_iter().enumerate() {
        let got = feature["geometry"]["coordinates"][i]
            .as_f64()
            .expect("coordinate");
        assert!((got - expected).abs() < 1e-9, "{got} vs {expected}");
    }
    // The source is unchanged.
    assert_eq!(
        accessmod_io::tiles::geojson(&source, 10, None).expect("geojson"),
        before
    );

    let unknown = PointMove {
        fid: fid + 100,
        ..moved
    };
    let error = project
        .relocate(3, &[unknown], &tags(&["again"]), &settings, &|| false)
        .expect_err("unknown feature");
    assert!(error.to_string().contains("not found"), "{error}");
    assert!(
        project
            .relocate(2, &[moved], &tags(&["x"]), &settings, &|| false)
            .is_err()
    );
    assert!(
        project
            .relocate(3, &[], &tags(&["x"]), &settings, &|| false)
            .is_err()
    );
}

#[test]
fn runs_are_retried_and_removed_with_or_without_their_outputs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    // Retry: only a run that ended without success; a new run, same job.
    let cancelled = project
        .start_named("travel-time", &travel_time_job(), &tags(&["tt"]), false)
        .expect("queue");
    assert!(project.cancel_queued(cancelled).expect("cancel"));
    let retried = project.retry_run(cancelled).expect("retry");
    assert_ne!(retried, cancelled);
    let done = project
        .execute_queued(retried, &settings, Monitor::QUIET)
        .expect("execute");
    assert_eq!(done.status, RunStatus::Done);
    assert_eq!(done.tags, ["tt"]);
    assert_eq!(done.job, {
        let mut job = project.run_record(cancelled).expect("run").job;
        job["output_dir"] = json!(format!("runs/{retried}"));
        job
    });
    assert!(project.retry_run(retried).is_err());

    // A queued or running run is not removed; nor its inputs.
    let queued = project
        .start_named("travel-time", &travel_time_job(), &[], false)
        .expect("queue");
    let err = project.remove_run(queued, false).expect_err("queued");
    assert!(err.to_string().contains("cancel it first"), "{err}");
    let err = project.remove_dataset(2).expect_err("read by a queued run");
    assert!(err.to_string().contains("is read by run"), "{err}");
    // Its outputs, read by the queued run? None here: they are removed with
    // the run on request, the directory with them.
    let outputs = done.datasets.clone();
    assert!(!outputs.is_empty());
    project.remove_run(retried, true).expect("remove");
    assert!(project.run_record(retried).is_err());
    assert!(outputs.iter().all(|id| project.dataset(*id).is_err()));
    assert!(!project.root().join(format!("runs/{retried}")).exists());
    assert!(project.cancel_queued(queued).expect("cancel"));

    // Without `outputs`, they stay, unlinked, with the directory.
    let kept = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &[],
            false,
            &settings,
            Monitor::QUIET,
        )
        .expect("run");
    project.remove_run(kept.id, false).expect("remove");
    assert!(project.run_record(kept.id).is_err());
    for id in &kept.datasets {
        let dataset = project.dataset(*id).expect("kept");
        assert!(project.root().join(&dataset.file).is_file());
    }
    assert!(project.root().join(&kept.output_dir).is_dir());
    // A run without outputs (cancelled) leaves no directory either.
    project.remove_run(cancelled, false).expect("remove");
    assert!(
        project
            .runs()
            .expect("runs")
            .iter()
            .all(|r| r.id != cancelled)
    );
}

#[test]
fn operations_are_runs_of_kind_manage_that_protect_what_they_read() {
    use accessmod_io::project::RunKind;
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let id = project
        .start_operation("archive", &json!({"name": "x"}), &[2, 3])
        .expect("queue");
    let record = project.run_record(id).expect("run");
    assert_eq!(
        (record.kind, record.status),
        (RunKind::Manage, RunStatus::Queued)
    );
    assert_eq!(record.inputs, [2, 3]);
    // An analysis never executes it; its inputs are held.
    let settings = settings(64 << 20, 1);
    assert!(
        project
            .execute_queued(id, &settings, Monitor::QUIET)
            .is_err()
    );
    assert!(project.remove_dataset(3).is_err());
    let running = project.claim_operation(id).expect("claim").expect("queued");
    assert_eq!(running.status, RunStatus::Running);
    assert!(project.claim_operation(id).expect("claim").is_none());
    let done = project
        .finish_operation(id, Ok(("archived".into(), json!({"file": "a.zip"}))))
        .expect("finish");
    assert_eq!(done.status, RunStatus::Done);
    assert_eq!(done.result, Some(json!({"file": "a.zip"})));
    project.remove_dataset(3).expect("no longer read");
    // A failed one records its error; a recorded one (that made the project)
    // is done at once.
    let failed = project
        .start_operation("export", &json!({}), &[])
        .expect("queue");
    project.claim_operation(failed).expect("claim");
    let error = accessmod_io::error::Error::Cancelled;
    let record = project
        .finish_operation(failed, Err(&error))
        .expect("finish");
    assert_eq!(record.status, RunStatus::Cancelled);
    let made = project
        .record_operation(
            "project-import",
            &json!({"file": "x.am6p"}),
            1,
            "imported",
            &json!(null),
        )
        .expect("record");
    assert_eq!((made.kind, made.status), (RunKind::Manage, RunStatus::Done));
}

#[test]
fn run_removal_keeps_directories_it_does_not_own_and_inputs_must_be_ready() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let run = project
        .run_named(
            "travel-time",
            &travel_time_job(),
            &[],
            false,
            &settings,
            Monitor::QUIET,
        )
        .expect("run");
    // A run from a project file may name any directory as its own.
    let db = rusqlite::Connection::open(project.root().join("project.sqlite")).expect("db");
    db.execute(
        "UPDATE runs SET output_dir = './datasets' WHERE id = ?1",
        [run.id],
    )
    .expect("update");
    project.remove_run(run.id, false).expect("remove");
    assert!(project.root().join("datasets/1.tif").is_file());
    // A dataset being removed is not recorded as an input.
    db.execute("UPDATE datasets SET state = 'deleting' WHERE id = 3", [])
        .expect("update");
    let err = project
        .start_operation("archive", &json!({}), &[2, 3])
        .expect_err("being removed");
    assert!(err.to_string().contains("dataset 3"), "{err}");
    assert!(
        project
            .runs()
            .expect("runs")
            .iter()
            .all(|r| r.analysis != "archive")
    );
}

/// The quick check finds what `prepare` finds, without loading the grid:
/// the same facility issues and class warnings, on the demo project.
#[test]
fn quick_checks_lint_like_prepare() {
    let dir = tempfile::tempdir().expect("tempdir");
    let settings = settings(64 << 20, 1);
    let project =
        accessmod_io::demo::create(&dir.path().join("demo"), &settings, &|| false).expect("demo");
    let first = |class| project.datasets(Some(class)).expect("datasets").remove(0);
    // Analyses take a merged land cover: the sample one, imported as such.
    let sample = project.root().join(&first(DataClass::LandCover).file);
    project
        .import(
            &sample,
            DataClass::LandCoverMerged,
            &tags(&["merged"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("merged land cover");
    let (dem, land_cover, facilities) = (
        first(DataClass::Dem),
        first(DataClass::LandCoverMerged),
        first(DataClass::Facility),
    );
    let Summary::Raster(preview) = &land_cover.summary else {
        panic!("raster summary");
    };
    let classes: Vec<i64> = preview
        .stats
        .values
        .as_ref()
        .expect("classes")
        .iter()
        .map(|v| v.value)
        .collect();
    let Summary::Vector(vector) = &facilities.summary else {
        panic!("vector summary");
    };
    let source = json!({"path": facilities.file, "id_field": vector.fields[0]});
    // Every class but the last two: one at 0 km/h, one without speed; and
    // a class absent from the map.
    let mut scenario: Vec<_> = classes[..classes.len() - 2]
        .iter()
        .map(|c| json!({"class": c, "mode": "walking", "speed_kmh": 5.0}))
        .collect();
    scenario
        .push(json!({"class": classes[classes.len() - 2], "mode": "walking", "speed_kmh": 0.0}));
    scenario.push(json!({"class": 9999, "mode": "walking", "speed_kmh": 5.0}));
    let base = json!({
        "schema_version": 1,
        "dem": dem.file,
        "land_cover": land_cover.file,
        "scenario": scenario,
        "method": "anisotropic",
        "neighbourhood": 8,
    });
    let mut travel_time = base.clone();
    travel_time["facilities"] = source.clone();
    travel_time["direction"] = json!("from_facilities");
    let mut referral = base.clone();
    referral["origins"] = source.clone();
    referral["destinations"] = source;
    for (analysis, mut job) in [("travel-time", travel_time), ("referral", referral)] {
        for policy in ["skip", "error"] {
            job["facility_issues"] = json!(policy);
            let quick = project.check_named(analysis, &job);
            let full = project.validate_named(analysis, &job, &settings, Monitor::QUIET);
            match (policy, quick, full) {
                ("skip", Ok(warnings), Ok(validation)) => {
                    let warnings = serde_json::to_value(&warnings).expect("json");
                    assert_eq!(warnings, validation["warnings"], "{analysis}");
                    let codes = warnings.to_string();
                    for code in [
                        "facility_skipped",
                        "scenario_speed_zero",
                        "classes_without_scenario",
                        "scenario_class_absent",
                    ] {
                        assert!(codes.contains(code), "{analysis}: {code} in {codes}");
                    }
                }
                ("error", Err(quick), Err(full)) => {
                    assert_eq!(quick.to_string(), full.to_string(), "{analysis}");
                }
                (policy, quick, full) => panic!("{analysis} {policy}: {quick:?} / {full:?}"),
            }
        }
    }
}

#[test]
fn active_runs_reserve_their_output_tags() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let scenario = tags(&["scenario", "a"]);
    let first = project
        .start_named("travel-time", &travel_time_job(), &scenario, false)
        .expect("queue");
    // Same output class and tags as a queued run: refused, overwrite or not.
    for overwrite in [false, true] {
        let err = project
            .start_named("travel-time", &travel_time_job(), &scenario, overwrite)
            .expect_err("reserved");
        assert!(
            err.to_string()
                .contains(&format!("run {first} already writes")),
            "{err}"
        );
    }
    // Other tags are free.
    let other = project
        .start_named(
            "travel-time",
            &travel_time_job(),
            &tags(&["scenario", "b"]),
            false,
        )
        .expect("other tags");
    assert!(project.cancel_queued(other).expect("cancel"));
    // A cancelled run frees its tags.
    assert!(project.cancel_queued(first).expect("cancel"));
    let again = project
        .start_named("travel-time", &travel_time_job(), &scenario, false)
        .expect("freed");
    project
        .execute_queued(again, &settings, Monitor::QUIET)
        .expect("execute");
}

#[test]
fn concurrent_starts_never_take_the_same_tags() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project_with_inputs(dir.path()).root().to_path_buf();
    for round in 0..10 {
        let tags = tags(&["race", &format!("r{round}")]);
        let barrier = std::sync::Barrier::new(2);
        let results: Vec<bool> = std::thread::scope(|scope| {
            let starts: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        let project = Project::open(&root).expect("open");
                        barrier.wait();
                        project
                            .start_named("travel-time", &travel_time_job(), &tags, false)
                            .is_ok()
                    })
                })
                .collect();
            starts
                .into_iter()
                .map(|h| h.join().expect("thread"))
                .collect()
        });
        assert_eq!(
            results.iter().filter(|ok| **ok).count(),
            1,
            "round {round}: {results:?}"
        );
    }
}

#[test]
fn float_land_cover_with_whole_values_is_written_as_int32() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let import = |name: &str, tag: &str, threads: usize| {
        project.import(
            &dir.path().join(name),
            DataClass::LandCover,
            &tags(&[tag]),
            &ImportOptions::default(),
            &settings(64 << 20, threads),
            &|| false,
        )
    };
    // As inAccessMod writes it: Float32, NaN for NoData, no NoData value.
    let mut values: Vec<f32> = (0..COLS * ROWS).map(|i| (i % 3 + 1) as f32).collect();
    values[0] = f32::NAN;
    values[1] = 1000.0;
    write_raster(
        &dir.path().join("lc_f32.tif"),
        COLS,
        ROWS,
        geo_transform(X0, Y0),
        values.clone(),
        None,
    );
    let record = import("lc_f32.tif", "f32", 1).expect("whole Float32");
    assert!(matches!(
        &record.origin,
        Origin::Import { converted_from: Some(from), .. } if from == "Float32"
    ));
    assert_eq!(
        record.conversion().as_deref(),
        Some("converted from Float32 to Int32: all values are whole")
    );
    let file = project.root().join(&record.file);
    let (written, nodata) = accessmod_io::fixtures::read_with_nodata::<i32>(&file);
    assert_eq!(nodata, Some(f64::from(i32::MAX)));
    assert_eq!(written[0], i32::MAX, "NaN is NoData");
    for (w, v) in written.iter().zip(&values).skip(1) {
        assert_eq!(*w, *v as i32);
    }
    // The same bytes whatever the threads.
    let again = import("lc_f32.tif", "f32_threads", 4).expect("again");
    assert_eq!(
        std::fs::read(&file).expect("read"),
        std::fs::read(project.root().join(&again.file)).expect("read")
    );

    // Float64 with a whole NoData value: kept.
    let mut values: Vec<f64> = vec![5.0; COLS * ROWS];
    values[3] = -9999.0;
    write_raster(
        &dir.path().join("lc_f64.tif"),
        COLS,
        ROWS,
        geo_transform(X0, Y0),
        values,
        Some(-9999.0),
    );
    let record = import("lc_f64.tif", "f64", 1).expect("whole Float64");
    let (written, nodata) =
        accessmod_io::fixtures::read_with_nodata::<i32>(&project.root().join(&record.file));
    assert_eq!(nodata, Some(-9999.0));
    assert_eq!((written[3], written[4]), (-9999, 5));

    // Integer land cover: not converted.
    let integer = project
        .datasets(Some(DataClass::LandCoverMerged))
        .expect("datasets");
    assert_eq!(integer[0].conversion(), None);
}

#[test]
fn float_land_cover_with_fractions_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let refused = |name: &str, values: Vec<f64>| {
        write_raster(
            &dir.path().join(name),
            COLS,
            ROWS,
            geo_transform(X0, Y0),
            values,
            None,
        );
        project
            .import(
                &dir.path().join(name),
                DataClass::LandCover,
                &tags(&[name.trim_end_matches(".tif")]),
                &ImportOptions::default(),
                &settings(64 << 20, 1),
                &|| false,
            )
            .expect_err("refused")
    };
    let mut values = vec![1.0; COLS * ROWS];
    values[COLS + 2] = 1.5;
    let err = refused("half.tif", values);
    assert_eq!(err.kind(), accessmod_io::error::ErrorKind::Validation);
    assert!(
        err.to_string()
            .contains("classes must be integers, Float64 holds 1.5 (row 1, column 2)"),
        "{err}"
    );
    let mut values = vec![1.0; COLS * ROWS];
    values[0] = 3e9;
    let err = refused("large.tif", values);
    assert!(err.to_string().contains("holds 3000000000"), "{err}");
    assert!(
        project
            .datasets(Some(DataClass::LandCover))
            .expect("datasets")
            .is_empty(),
        "nothing is left"
    );
}
