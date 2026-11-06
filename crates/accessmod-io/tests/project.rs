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
        .create_archive("e2e", &datasets, &[run.id], &|| false)
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
        &|| false,
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
        .create_archive("", &run.datasets, &[run.id], &|| false)
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
    assert!(project.create_archive("", &[], &[], &|| false).is_err());
    assert!(
        project
            .create_archive("bad name!", &[1], &[], &|| false)
            .is_err()
    );

    // An input removed since the run: every remaining file archived, still
    // not replayable (the job names the removed file).
    project.remove_dataset(2).expect("remove land cover");
    let mut rest: Vec<i64> = run.inputs.iter().chain(&run.datasets).copied().collect();
    rest.retain(|&id| id != 2);
    let gone = project
        .create_archive("gone", &rest, &[run.id], &|| false)
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
            .create_archive("", &[3], &[], &|| false)
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
