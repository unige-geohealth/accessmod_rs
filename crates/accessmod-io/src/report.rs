//! `run.json`: the parts every analysis report shares.
//!
//! A report has a deterministic part (`run`: job, inputs, grid, results,
//! warnings, deviations), identical whatever the execution settings, and an
//! execution part (`execution`: settings, plan, durations, versions).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::resources::{self, ExecutionSettings, ResourcePlan};

/// Version of the `run.json` document.
pub const RUN_REPORT_VERSION: u32 = 1;

/// `run.json` of an analysis: `R` is its deterministic part, `X` the
/// analysis-specific execution details.
#[derive(Debug, Clone, Serialize)]
pub struct Report<R, X> {
    /// Version of this document.
    pub report_version: u32,
    /// Deterministic part.
    pub run: R,
    /// Execution part.
    pub execution: Execution<X>,
}

impl<R, X> Report<R, X> {
    /// Report of the current version.
    pub fn new(run: R, execution: Execution<X>) -> Self {
        Self {
            report_version: RUN_REPORT_VERSION,
            run,
            execution,
        }
    }
}

/// Execution part of `run.json`: varies with host and settings.
#[derive(Debug, Clone, Serialize)]
pub struct Execution<X> {
    /// Settings used.
    pub settings: ExecutionSettings,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Seconds per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
    /// Analysis-specific details (cache counters, workers).
    #[serde(flatten)]
    pub details: X,
    /// Peak resident memory, bytes, where reported by the platform.
    pub peak_rss_bytes: Option<u64>,
    /// Versions of the engine and libraries.
    pub versions: BTreeMap<&'static str, String>,
}

impl<X> Execution<X> {
    /// Execution part at the end of a run: peak memory and versions are
    /// read now.
    pub fn now(
        settings: &ExecutionSettings,
        plan: ResourcePlan,
        durations_s: BTreeMap<&'static str, f64>,
        details: X,
    ) -> Self {
        Self {
            settings: settings.clone(),
            plan,
            durations_s,
            details,
            peak_rss_bytes: resources::peak_rss(),
            versions: versions(),
        }
    }
}

/// A non-blocking problem, reported in `run.json` and on stderr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// A documented difference from AccessMod 5 that applies to this run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Deviation {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// What differs and why.
    pub message: &'static str,
}

/// Size, checksum and path of an input file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fingerprint {
    /// Role of the file in the job.
    pub role: &'static str,
    /// Path.
    pub path: PathBuf,
    /// Size, bytes.
    pub bytes: u64,
    /// SHA-256 of the file content (main file only for multi-file formats).
    pub sha256: String,
}

/// Grid description for the report.
#[derive(Debug, Clone, Serialize)]
pub struct GridReport {
    /// Columns.
    pub cols: usize,
    /// Rows.
    pub rows: usize,
    /// GDAL geotransform.
    pub geo_transform: [f64; 6],
    /// CRS as WKT.
    pub crs_wkt: String,
}

impl GridReport {
    /// Description of `grid`.
    pub fn of(grid: &GridSpec) -> Self {
        Self {
            cols: grid.cols,
            rows: grid.rows,
            geo_transform: grid.geo_transform,
            crs_wkt: grid.srs.to_wkt().unwrap_or_default(),
        }
    }
}

/// Fingerprint of one file.
pub fn sha256_file(role: &'static str, path: &Path) -> Result<Fingerprint> {
    let mut file = std::fs::File::open(path).map_err(Error::file(path))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut bytes = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(Error::file(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        bytes += n as u64;
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Fingerprint {
        role,
        path: path.to_path_buf(),
        bytes,
        sha256,
    })
}

/// Fingerprints of `(role, path)` input files, in order.
pub fn fingerprints(
    files: &[(&'static str, &Path)],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Fingerprint>> {
    files
        .iter()
        .map(|&(role, path)| {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            sha256_file(role, path)
        })
        .collect()
}

/// Versions of the engine, GDAL and PROJ.
pub fn versions() -> BTreeMap<&'static str, String> {
    let mut map = BTreeMap::new();
    map.insert("accessmod", env!("CARGO_PKG_VERSION").to_owned());
    map.insert("gdal", gdal::version::version_info("RELEASE_NAME"));
    let proj = gdal::version::version_info("BUILD_INFO")
        .lines()
        .find_map(|l| l.strip_prefix("PROJ_RUNTIME_VERSION=").map(str::to_owned))
        .unwrap_or_default();
    map.insert("proj", proj);
    map
}
