//! `accessmod doctor`: is this machine ready to run analyses?

use std::path::PathBuf;

use accessmod_io::resources;
use accessmod_io::travel_time::versions;
use gdal::DriverManager;
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use serde_json::json;

use crate::{Failure, print_json};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Print a JSON document on stdout.
    #[arg(long)]
    json: bool,
    /// Scratch directory to check (default: system temporary directory).
    #[arg(long, value_name = "DIR")]
    tmp_dir: Option<PathBuf>,
}

/// One check: name, success and detail.
fn check(name: &str, ok: bool, detail: String) -> serde_json::Value {
    json!({"name": name, "ok": ok, "detail": detail})
}

/// PROJ can build a projected CRS and transform a point.
fn proj_check() -> (bool, String) {
    let result = (|| -> Result<(f64, f64), gdal::errors::GdalError> {
        let mut utm = SpatialRef::from_epsg(32632)?;
        let mut wgs84 = SpatialRef::from_epsg(4326)?;
        utm.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
        wgs84.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
        let transform = CoordTransform::new(&wgs84, &utm)?;
        let (mut x, mut y) = ([9.0], [45.0]);
        transform.transform_coords(&mut x, &mut y, &mut [])?;
        Ok((x[0], y[0]))
    })();
    match result {
        Ok((x, y)) => (
            (x - 500_000.0).abs() < 1.0,
            format!("EPSG:4326 (9E, 45N) -> EPSG:32632 ({x:.1}, {y:.1})"),
        ),
        Err(e) => (
            false,
            format!("PROJ database or transformation unavailable: {e}"),
        ),
    }
}

pub fn run(args: &Args) -> Result<(), Failure> {
    let tmp_dir = args.tmp_dir.clone().unwrap_or_else(std::env::temp_dir);
    let mut checks = Vec::new();

    for driver in ["GTiff", "GPKG", "ESRI Shapefile", "GeoJSON"] {
        let ok = DriverManager::get_driver_by_name(driver).is_ok();
        let detail = if ok { "available" } else { "missing" };
        checks.push(check(&format!("driver {driver}"), ok, detail.to_owned()));
    }
    let (ok, detail) = proj_check();
    checks.push(check("proj", ok, detail));

    let probe = tmp_dir.join(format!(".accessmod-doctor-{}", std::process::id()));
    let writable = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    checks.push(check(
        "scratch directory",
        writable,
        format!(
            "{} {}",
            tmp_dir.display(),
            if writable {
                "is writable"
            } else {
                "is not writable"
            }
        ),
    ));

    let free_disk = resources::free_disk(&tmp_dir);
    let total_memory = resources::total_memory();
    let available_memory = resources::available_memory();
    let ok = checks.iter().all(|c| c["ok"] == true);

    let report = json!({
        "status": if ok { "ok" } else { "fail" },
        "versions": versions(),
        "checks": checks,
        "host": {
            "cpus": resources::cpus(),
            "total_memory_bytes": total_memory,
            "available_memory_bytes": available_memory,
            "scratch_dir": tmp_dir,
            "scratch_free_bytes": free_disk,
        },
        "defaults": {
            "memory_bytes": resources::default_memory_budget(),
            "threads": resources::default_threads(),
        },
    });
    if args.json {
        print_json(&report);
    } else {
        let v = versions();
        eprintln!(
            "accessmod {}  GDAL {}  PROJ {}",
            v.get("accessmod").map_or("?", String::as_str),
            v.get("gdal").map_or("?", String::as_str),
            v.get("proj").map_or("?", String::as_str)
        );
        for c in report["checks"].as_array().into_iter().flatten() {
            let mark = if c["ok"] == true { "ok  " } else { "FAIL" };
            eprintln!(
                "{mark} {}: {}",
                c["name"].as_str().unwrap_or(""),
                c["detail"].as_str().unwrap_or("")
            );
        }
        eprintln!(
            "host: {} CPUs, {:.1} GiB RAM ({:.1} GiB available), scratch free {}",
            resources::cpus(),
            total_memory as f64 / f64::from(1u32 << 30),
            available_memory as f64 / f64::from(1u32 << 30),
            free_disk.map_or_else(
                || "unknown".to_owned(),
                |b| format!("{:.1} GiB", b as f64 / f64::from(1u32 << 30))
            )
        );
    }
    if ok { Ok(()) } else { Err(Failure { code: 3 }) }
}
