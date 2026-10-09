//! The `accessmod` binary: exit codes, stdout JSON, direct and job modes.

use std::path::Path;
use std::process::{Command, Output};

use gdal::DriverManager;
use gdal::raster::Buffer;
use gdal::spatial_ref::SpatialRef;
use gdal::vector::{
    Feature, FieldValue, Geometry, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType,
};

const EPSG: u32 = 32632;

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

fn write_raster<T: gdal::raster::GdalType + Copy>(path: &Path, value: T) {
    let gt = [500_000.0, 100.0, 0.0, 5_000_000.0, 0.0, -100.0];
    let mut ds = DriverManager::get_driver_by_name("GTiff")
        .expect("GTiff")
        .create_with_band_type::<T, _>(path, 4, 3, 1)
        .expect("create");
    ds.set_geo_transform(&gt).expect("gt");
    ds.set_spatial_ref(&SpatialRef::from_epsg(EPSG).expect("srs"))
        .expect("srs");
    let mut band = ds.rasterband(1).expect("band");
    band.write((0, 0), (4, 3), &mut Buffer::new((4, 3), vec![value; 12]))
        .expect("write");
}

/// 4 x 3 flat landscape, class 1 everywhere, one facility in cell (1, 0).
fn fixture(dir: &Path) {
    write_raster(&dir.join("dem.tif"), 0.0f32);
    write_raster(&dir.join("lc.tif"), 1u8);
    let srs = SpatialRef::from_epsg(EPSG).expect("srs");
    let mut ds = DriverManager::get_driver_by_name("GPKG")
        .expect("GPKG")
        .create_vector_only(dir.join("hf.gpkg"))
        .expect("create");
    let layer = ds
        .create_layer(LayerOptions {
            name: "hf",
            srs: Some(&srs),
            ty: OGRwkbGeometryType::wkbPoint,
            options: None,
        })
        .expect("layer");
    layer
        .create_defn_fields(&[("code", OGRFieldType::OFTString)])
        .expect("field");
    let field = layer.defn().field_index("code").expect("field");
    let mut feature = Feature::new(layer.defn()).expect("feature");
    feature
        .set_geometry(Geometry::from_wkt("POINT (500050 4999850)").expect("point"))
        .expect("geometry");
    feature
        .set_field(field, &FieldValue::StringValue("HC-1".into()))
        .expect("field");
    feature.create(&layer).expect("create");
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
