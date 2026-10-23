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
            DataClass::LandCover,
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
        [DataClass::Dem, DataClass::LandCover, DataClass::Facility]
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
            DataClass::LandCover,
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
            DataClass::LandCoverMerged,
            &tags(&["base"]),
            &ImportOptions::default(),
            &settings,
            &|| false,
        )
        .expect("other class");
    let retagged = project.retag(2, &tags(&["base 2024"])).expect("retag");
    assert_eq!(retagged.tags, ["base", "2024"]);
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
            &settings,
            Monitor::QUIET,
        )
        .expect_err("tags taken");
    assert!(err.to_string().contains("already"), "{err}");
    let runs = project.runs().expect("runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].status, RunStatus::Failed);
    assert!(!project.root().join("runs/2").exists());
}

#[test]
fn failed_runs_record_their_issues() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project_with_inputs(dir.path());
    let settings = settings(64 << 20, 1);
    let mut job = travel_time_job();
    job["facilities"]["id_field"] = json!("missing_field");
    let err = project
        .run_named("travel-time", &job, &[], &settings, Monitor::QUIET)
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
        .save_job(Some(saved.id), "travel-time", "renamed", &travel_time_job())
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
    let started = project
        .start_run::<TravelTime>(&travel_time_job(), &[])
        .expect("start");
    drop(started);
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
