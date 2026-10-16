//! The `accessmod` binary: exit codes, stdout JSON, direct and job modes.

use std::path::Path;
use std::process::{Command, Output};

use accessmod_io::fixtures::{EPSG, X0, Y0, geo_transform, write_layer, write_raster};
use gdal::vector::{FieldValue, OGRFieldType};

fn accessmod(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_accessmod"))
        .args(args)
        .env("ACCESSMOD_LOG", "warn")
        .output()
        .expect("run accessmod")
}

fn stdout_json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// 4 x 3 flat landscape, class 1 everywhere, one facility in cell (1, 0).
fn fixture(dir: &Path) {
    let gt = geo_transform(X0, Y0);
    write_raster(&dir.join("dem.tif"), 4, 3, gt, vec![0.0f32; 12], None);
    write_raster(&dir.join("lc.tif"), 4, 3, gt, vec![1u8; 12], None);
    write_layer(
        &dir.join("hf.gpkg"),
        "hf",
        EPSG,
        ("code", OGRFieldType::OFTString),
        &[(
            Some(FieldValue::StringValue("HC-1".into())),
            "POINT (500050 4999850)",
        )],
    );
}

fn direct_args<'a>(dir: &'a str, out: &'a str) -> Vec<String> {
    [
        "travel-time",
        "run",
        "--json",
        "--memory",
        "256M",
        "--dem",
        &format!("{dir}/dem.tif"),
        "--land-cover",
        &format!("{dir}/lc.tif"),
        "--facilities",
        &format!("{dir}/hf.gpkg"),
        "--id-field",
        "code",
        "--class",
        "1=motorized:5",
        "--method",
        "isotropic",
        "--direction",
        "from_facilities",
        "--neighbourhood",
        "8",
        "--output-dir",
        out,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[test]
fn schema_is_json() {
    let output = accessmod(&["travel-time", "schema"]);
    assert!(output.status.success());
    let schema = stdout_json(&output);
    assert!(schema["properties"]["dem"].is_object());
}

#[test]
fn doctor_reports_json() {
    let output = accessmod(&["doctor", "--json"]);
    let report = stdout_json(&output);
    assert_eq!(report["status"], "ok", "{report:#}");
    assert!(report["versions"]["gdal"].is_string());
}

#[test]
fn direct_mode_runs_and_reports() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let dir = tmp.path().to_string_lossy().into_owned();
    let out = format!("{dir}/out");
    let args = direct_args(&dir, &out);
    let output = accessmod(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = stdout_json(&output);
    assert_eq!(report["status"], "ok");
    assert_eq!(report["run"]["result"]["reached_cells"], 12);
    assert!(Path::new(&out).join("job.resolved.json").is_file());

    // Same command again: the output directory is no longer empty.
    let output = accessmod(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(output.status.code(), Some(2));
    let error = stdout_json(&output);
    assert_eq!(error["kind"], "validation");

    // The resolved job replays the run.
    let replay = format!("{dir}/replay");
    let mut job: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&out).join("job.resolved.json")).expect("read"),
    )
    .expect("json");
    job["output_dir"] = serde_json::Value::from(replay.clone());
    let job_path = format!("{dir}/job.json");
    std::fs::write(&job_path, job.to_string()).expect("write job");
    let output = accessmod(&["travel-time", "run", "--json", "--job", &job_path]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for name in [
        "travel_time_seconds.tif",
        "nearest_facility.tif",
        "path_distance_m.tif",
    ] {
        let a = std::fs::read(Path::new(&out).join(name)).expect("read");
        let b = std::fs::read(Path::new(&replay).join(name)).expect("read");
        assert!(a == b, "{name} differs on replay");
    }
}

#[test]
fn validation_errors_exit_2_with_every_issue() {
    let output = accessmod(&["travel-time", "validate", "--json", "--dem", "missing.tif"]);
    assert_eq!(output.status.code(), Some(2));
    let error = stdout_json(&output);
    let issues = error["issues"].as_array().expect("issues");
    assert!(issues.len() >= 5, "{issues:?}");
}

#[test]
fn job_and_direct_parameters_conflict() {
    let output = accessmod(&["travel-time", "run", "--job", "j.json", "--dem", "d.tif"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "clap usage errors exit with 2"
    );
}

#[test]
fn inspect_describes_rasters_and_layers() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let output = accessmod(&[
        "dataset",
        "inspect",
        "--json",
        &tmp.path().join("dem.tif").to_string_lossy(),
    ]);
    let report = stdout_json(&output);
    assert_eq!(report["raster"]["cols"], 4);
    assert_eq!(report["raster"]["usable_as_dem"], true);
    let output = accessmod(&[
        "dataset",
        "inspect",
        "--json",
        &tmp.path().join("hf.gpkg").to_string_lossy(),
    ]);
    let report = stdout_json(&output);
    assert_eq!(report["layers"][0]["features"], 1);
}

#[test]
fn referral_runs_from_a_job() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let dir = tmp.path().to_string_lossy().into_owned();
    let job = format!(
        r#"{{"schema_version": 1, "dem": "{dir}/dem.tif", "land_cover": "{dir}/lc.tif",
            "origins": {{"path": "{dir}/hf.gpkg", "id_field": "code"}},
            "destinations": {{"path": "{dir}/hf.gpkg", "id_field": "code"}},
            "scenario": [{{"class": 1, "mode": "motorized", "speed_kmh": 5}}],
            "method": "isotropic", "neighbourhood": 8, "self_pairs": true,
            "output_dir": "{dir}/ref"}}"#
    );
    let job_path = format!("{dir}/referral.json");
    std::fs::write(&job_path, job).expect("write job");
    let output = accessmod(&["referral", "run", "--json", "--job", &job_path]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = stdout_json(&output);
    assert_eq!(report["run"]["result"]["pairs"], 1);
    let csv = std::fs::read_to_string(format!("{dir}/ref/referral.csv")).expect("csv");
    assert_eq!(
        csv,
        "origin_id,destination_id,time_s,distance_m\nHC-1,HC-1,0,0\n"
    );
    let schema = stdout_json(&accessmod(&["referral", "schema"]));
    assert!(schema["properties"]["destinations"].is_object());
}

#[test]
fn merge_land_cover_runs_from_a_job() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let dir = tmp.path().to_string_lossy().into_owned();
    // The facility layer has no class field: stack the land cover alone,
    // then as a conflicting pair.
    let job = |stack: &str, out: &str| {
        format!(
            r#"{{"schema_version": 1, "dem": "{dir}/dem.tif", "stack": [{stack}],
                "output_dir": "{dir}/{out}"}}"#
        )
    };
    let lc = format!(r#"{{"type": "land_cover", "path": "{dir}/lc.tif"}}"#);
    let job_path = format!("{dir}/merge.json");
    std::fs::write(&job_path, job(&lc, "merged")).expect("write job");
    let output = accessmod(&["merge-land-cover", "run", "--json", "--job", &job_path]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = stdout_json(&output);
    assert_eq!(report["run"]["result"]["classified_cells"], 12);
    let csv = std::fs::read_to_string(format!("{dir}/merged/classes.csv")).expect("csv");
    assert_eq!(csv, "class,label,stack_item,cells\n1,,1,12\n");

    std::fs::write(&job_path, job(&format!("{lc}, {lc}"), "conflict")).expect("write job");
    let output = accessmod(&["merge-land-cover", "validate", "--json", "--job", &job_path]);
    assert_eq!(output.status.code(), Some(2));
    let error = stdout_json(&output);
    assert!(
        error
            .to_string()
            .contains("class 1 comes from stack items 1"),
        "{error}"
    );

    let schema = stdout_json(&accessmod(&["merge-land-cover", "schema"]));
    assert!(schema["properties"]["stack"].is_object());
}
