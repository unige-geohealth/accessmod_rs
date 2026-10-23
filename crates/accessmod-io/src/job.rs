//! Travel-time job, schema version 1.
//!
//! A job is the scientific description of a run: inputs, scenario, method
//! and outputs. Execution settings (memory, threads, scratch directory) are
//! not part of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use accessmod_core::cost::{CellMobility, Direction, Method, Neighbourhood};
use accessmod_core::mobility::{MobilityModel, TransportMode, bicycle};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::output;

/// Current job schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Travel-time job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TravelTimeJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Digital elevation model (GeoTIFF or any GDAL raster). It defines the
    /// grid: CRS, resolution, origin and extent. Always required.
    #[schemars(extend("x-accessmod-class" = ["rDem"]))]
    pub dem: PathBuf,
    /// Land-cover raster with integer classes, on the DEM lattice.
    #[schemars(extend("x-accessmod-class" = ["rLandCover", "rLandCoverMerged"]))]
    pub land_cover: PathBuf,
    /// Facility point layer.
    #[schemars(extend("x-accessmod-class" = ["vFacility"]))]
    pub facilities: FacilitySource,
    /// Mobility of each land-cover class. Classes not listed are barriers.
    pub scenario: Vec<ScenarioEntry>,
    /// Travel-time method.
    pub method: MethodName,
    /// Direction of travel.
    pub direction: Direction,
    /// Neighbours reachable in one step: 8 or 16.
    pub neighbourhood: Neighbourhood,
    /// Inclusive travel-time limit, seconds; cells beyond are not reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_s: Option<f64>,
    /// Named mobility model (anisotropic method).
    #[serde(default)]
    pub mobility_model: MobilityModel,
    /// What to do with facilities outside the grid or on a barrier.
    #[serde(default)]
    pub facility_issues: FacilityIssues,
    /// Optional outputs.
    #[serde(default)]
    pub outputs: OutputOptions,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

/// Facility point layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FacilitySource {
    /// Vector dataset (GeoPackage, Shapefile, GeoJSON, …).
    pub path: PathBuf,
    /// Layer name; the first layer when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Field holding the facility identifier.
    pub id_field: String,
    /// Optional OGR SQL attribute filter selecting facilities, e.g.
    /// `"type = 'hospital'"`; all features when omitted.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// Mobility of one land-cover class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioEntry {
    /// Land-cover class value.
    pub class: i64,
    /// Transport mode.
    pub mode: TransportMode,
    /// Flat-ground speed, km/h, > 0 (≤ 100 for bicycling).
    pub speed_kmh: f64,
    /// Optional human label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Travel-time method name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MethodName {
    /// Scenario speed only; slope and mode ignored.
    Isotropic,
    /// Mobility model with slope and transport mode.
    Anisotropic,
}

/// Handling of facilities outside the grid, on a barrier or without a
/// usable geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FacilityIssues {
    /// Fail validation, listing every problem facility.
    #[default]
    Error,
    /// Skip them and list them in the warnings.
    Skip,
}

/// Optional outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputOptions {
    /// Write `direction.tif` (step code that reached each cell).
    #[serde(default)]
    pub direction: bool,
}

/// A job document: JSON, versioned, with paths relative to its file.
pub trait JobFile: Sized + DeserializeOwned + JsonSchema {
    /// Paths made absolute against `base` (when relative); see
    /// [`absolutize`].
    fn resolve_paths(self, base: &Path) -> Self;

    /// Parse a job from JSON.
    fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|e| Error::invalid(format!("job JSON: {e}")))
    }

    /// Read and parse a job file; relative paths are resolved against the
    /// file's directory.
    fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(Error::file(path))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Ok(Self::from_json(&text)?.resolve_paths(base))
    }

    /// JSON schema of the job document.
    fn schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Self))
            .expect("a generated schema serialises to JSON")
    }
}

/// Make `path` absolute against `base` when it is relative.
pub fn absolutize(path: &mut PathBuf, base: &Path) {
    if !path.is_absolute() {
        *path = base.join(&*path);
    }
}

/// Every repetition in `values`: (value, previous position, position),
/// 0-based, in order of appearance.
pub fn repeated<T: Ord + Copy>(values: impl IntoIterator<Item = T>) -> Vec<(T, usize, usize)> {
    let mut last = BTreeMap::new();
    values
        .into_iter()
        .enumerate()
        .filter_map(|(i, value)| last.insert(value, i).map(|previous| (value, previous, i)))
        .collect()
}

/// Problem with a job's `schema_version`, if any.
pub fn check_schema_version(schema_version: u32) -> Option<String> {
    (schema_version != SCHEMA_VERSION).then(|| {
        format!("schema_version is {schema_version}, this engine reads version {SCHEMA_VERSION}")
    })
}

/// Problems found before reading any input: the output directory must not
/// exist or be empty, and every named input file must exist.
pub fn check_files<'a>(
    output_dir: &Path,
    inputs: impl IntoIterator<Item = (String, &'a Path)>,
) -> Vec<String> {
    let mut issues = match output::check_output_dir(output_dir) {
        Err(Error::Validation { issues }) => issues,
        _ => Vec::new(),
    };
    for (name, path) in inputs {
        if !path.exists() {
            issues.push(format!("{name}: {} does not exist", path.display()));
        }
    }
    issues
}

impl JobFile for TravelTimeJob {
    fn resolve_paths(mut self, base: &Path) -> Self {
        for path in [
            &mut self.dem,
            &mut self.land_cover,
            &mut self.facilities.path,
            &mut self.output_dir,
        ] {
            absolutize(path, base);
        }
        self
    }
}

impl TravelTimeJob {
    /// Solver method.
    pub fn method(&self) -> Method {
        match self.method {
            MethodName::Isotropic => Method::Isotropic,
            MethodName::Anisotropic => Method::Anisotropic(self.mobility_model),
        }
    }

    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues = check_common(self.schema_version, &self.scenario, self.limit_s);
        if self.facilities.id_field.trim().is_empty() {
            issues.push("facilities.id_field is empty".into());
        }
        issues
    }

    /// Scenario as solver slots: class value → slot, and slot → mobility.
    /// Call after [`TravelTimeJob::check`] succeeded.
    pub fn scenario_table(&self) -> (BTreeMap<i64, u16>, Vec<CellMobility>) {
        scenario_table(&self.scenario)
    }
}

/// Problems with the fields every analysis job shares: schema version,
/// scenario and limit.
pub fn check_common(
    schema_version: u32,
    scenario: &[ScenarioEntry],
    limit_s: Option<f64>,
) -> Vec<String> {
    let mut issues: Vec<String> = check_schema_version(schema_version).into_iter().collect();
    if scenario.is_empty() {
        issues.push("scenario is empty: every cell would be a barrier".into());
    }
    let repeats: BTreeMap<usize, usize> = repeated(scenario.iter().map(|e| e.class))
        .into_iter()
        .map(|(_, previous, i)| (i, previous))
        .collect();
    for (i, entry) in scenario.iter().enumerate() {
        if let Some(previous) = repeats.get(&i) {
            issues.push(format!(
                "scenario class {} is listed twice (entries {} and {})",
                entry.class,
                previous + 1,
                i + 1
            ));
        }
        if CellMobility::new(entry.mode, entry.speed_kmh).is_none() {
            issues.push(format!(
                "scenario class {}: speed_kmh must be finite and > 0, got {} \
                 (leave a class out of the scenario to make it a barrier)",
                entry.class, entry.speed_kmh
            ));
        }
        if entry.mode == TransportMode::Bicycling && entry.speed_kmh > bicycle::SPEED_MAX_KMH {
            issues.push(format!(
                "scenario class {}: bicycling speed must not exceed {} km/h, got {}",
                entry.class,
                bicycle::SPEED_MAX_KMH,
                entry.speed_kmh
            ));
        }
    }
    if scenario.len() >= usize::from(accessmod_core::solver::InputCell::BARRIER) {
        issues.push(format!(
            "scenario has {} classes, at most 65534 are supported",
            scenario.len()
        ));
    }
    if let Some(limit) = limit_s
        && !(limit.is_finite() && limit > 0.0)
    {
        issues.push(format!("limit_s must be finite and > 0, got {limit}"));
    }
    issues
}

/// Scenario as solver slots: class value → slot, and slot → mobility
/// (first entry wins for a duplicated class; [`check_common`] reports it).
pub fn scenario_table(scenario: &[ScenarioEntry]) -> (BTreeMap<i64, u16>, Vec<CellMobility>) {
    let mut slots = BTreeMap::new();
    let mut table = Vec::with_capacity(scenario.len());
    for entry in scenario {
        if let (Some(mobility), Ok(slot)) = (
            CellMobility::new(entry.mode, entry.speed_kmh),
            u16::try_from(table.len()),
        ) && !slots.contains_key(&entry.class)
        {
            slots.insert(entry.class, slot);
            table.push(mobility);
        }
    }
    (slots, table)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "schema_version": 1,
        "dem": "dem.tif",
        "land_cover": "lc.tif",
        "facilities": {"path": "hf.gpkg", "id_field": "id"},
        "scenario": [
            {"class": 1, "mode": "walking", "speed_kmh": 5},
            {"class": 2, "mode": "motorized", "speed_kmh": 60, "label": "road"}
        ],
        "method": "anisotropic",
        "direction": "from_facilities",
        "neighbourhood": 16,
        "output_dir": "out"
    }"#;

    #[test]
    fn minimal_job_parses_with_defaults() {
        let job = TravelTimeJob::from_json(MINIMAL).expect("valid job");
        assert!(job.check().is_empty(), "{:?}", job.check());
        assert_eq!(job.mobility_model, MobilityModel::Accessmod5V1);
        assert_eq!(job.facility_issues, FacilityIssues::Error);
        assert!(!job.outputs.direction);
        assert_eq!(
            job.method(),
            Method::Anisotropic(MobilityModel::Accessmod5V1)
        );
        let (slots, table) = job.scenario_table();
        assert_eq!(slots.get(&2), Some(&1));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = MINIMAL.replace("\"output_dir\"", "\"outptu_dir\": \"x\", \"output_dir\"");
        assert!(TravelTimeJob::from_json(&text).is_err());
    }

    #[test]
    fn semantic_problems_are_all_reported() {
        let mut job = TravelTimeJob::from_json(MINIMAL).expect("valid job");
        job.schema_version = 2;
        job.scenario.push(ScenarioEntry {
            class: 1,
            mode: TransportMode::Bicycling,
            speed_kmh: 120.0,
            label: None,
        });
        job.scenario.push(ScenarioEntry {
            class: 3,
            mode: TransportMode::Walking,
            speed_kmh: 0.0,
            label: None,
        });
        job.limit_s = Some(-1.0);
        let issues = job.check();
        assert_eq!(issues.len(), 5, "{issues:#?}");
    }

    #[test]
    fn relative_paths_resolve_against_the_job_directory() {
        let job = TravelTimeJob::from_json(MINIMAL)
            .expect("valid job")
            .resolve_paths(Path::new("/data/jobs"));
        assert_eq!(job.dem, PathBuf::from("/data/jobs/dem.tif"));
        assert_eq!(job.output_dir, PathBuf::from("/data/jobs/out"));
    }

    #[test]
    fn schema_lists_required_fields() {
        let schema = TravelTimeJob::schema();
        let required = schema["required"].as_array().expect("required list");
        assert!(required.iter().any(|v| v == "dem"));
        assert!(!required.iter().any(|v| v == "limit_s"));
    }
}
