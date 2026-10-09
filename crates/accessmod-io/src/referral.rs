//! Referral analysis: travel time and path distance between facilities
//! (`docs/referral.md`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::cost::{CellMobility, Method, Neighbourhood};
use accessmod_core::mobility::MobilityModel;
use accessmod_core::referral::{Pair, ReferralConfig, WorkerBudget, referral};
use accessmod_core::solver::{InputCell, SolverCell};
use accessmod_core::store::{TileGeometry, TiledGrid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::facilities::{FacilityRecord, LoadedFacilities, load_facilities};
use crate::grid::GridSpec;
use crate::inputs::InputSummary;
use crate::job::{
    FacilityIssues, FacilitySource, MethodName, ScenarioEntry, check_common, scenario_table,
};
use crate::output::{self, Staging};
use crate::resources;
use crate::travel_time::{
    Deviation, ExecutionSettings, Fingerprint, GridReport, Landscape, ResourcePlan, Warning,
    landscape_warnings, load_landscape, screen_facilities, sha256_file, versions,
};

/// Referral job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferralJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Digital elevation model; it defines the grid.
    pub dem: PathBuf,
    /// Land-cover raster with integer classes, on the DEM lattice.
    pub land_cover: PathBuf,
    /// Facilities travel starts from.
    pub origins: FacilitySource,
    /// Facilities travel goes to (may be the same layer as the origins).
    pub destinations: FacilitySource,
    /// Mobility of each land-cover class. Classes not listed are barriers.
    pub scenario: Vec<ScenarioEntry>,
    /// Travel-time method.
    pub method: MethodName,
    /// Neighbours reachable in one step: 8 or 16.
    pub neighbourhood: Neighbourhood,
    /// Inclusive travel-time limit, seconds; pairs beyond are not reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_s: Option<f64>,
    /// Named mobility model (anisotropic method).
    #[serde(default)]
    pub mobility_model: MobilityModel,
    /// Only the nearest destination (by time) of each origin.
    #[serde(default)]
    pub nearest_only: bool,
    /// Keep pairs from a facility to itself (same feature of the same
    /// layer); AccessMod 5 drops them.
    #[serde(default)]
    pub self_pairs: bool,
    /// What to do with facilities outside the grid or on a barrier.
    #[serde(default)]
    pub facility_issues: FacilityIssues,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

impl ReferralJob {
    /// Parse a job from JSON.
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|e| Error::invalid(format!("job JSON: {e}")))
    }

    /// Read and parse a job file; relative paths are resolved against the
    /// file's directory.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(Error::file(path))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Ok(Self::from_json(&text)?.resolve_paths(base))
    }

    /// JSON schema of the job document.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(ReferralJob))
            .expect("a generated schema serialises to JSON")
    }

    /// Paths made absolute against `base` (when relative).
    pub fn resolve_paths(mut self, base: &Path) -> Self {
        let absolute = |p: &Path| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                base.join(p)
            }
        };
        self.dem = absolute(&self.dem);
        self.land_cover = absolute(&self.land_cover);
        self.origins.path = absolute(&self.origins.path);
        self.destinations.path = absolute(&self.destinations.path);
        self.output_dir = absolute(&self.output_dir);
        self
    }

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
        for (role, source) in [
            ("origins", &self.origins),
            ("destinations", &self.destinations),
        ] {
            if source.id_field.trim().is_empty() {
                issues.push(format!("{role}.id_field is empty"));
            }
        }
        issues
    }
}

/// Inputs loaded and validated, ready to compute.
#[derive(Debug)]
pub struct PreparedReferral {
    /// Job with resolved paths.
    pub job: ReferralJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Solver inputs.
    pub inputs: TiledGrid<InputCell>,
    /// Scenario slots.
    pub scenario: Vec<CellMobility>,
    /// Origins used and rejected.
    pub origins: LoadedFacilities,
    /// Destinations used and rejected.
    pub destinations: LoadedFacilities,
    /// Input counts.
    pub summary: InputSummary,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Input files.
    pub fingerprints: Vec<Fingerprint>,
    /// Seconds spent per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
}

/// Validate the job against its inputs and load them.
pub fn prepare(
    job: ReferralJob,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedReferral> {
    let started = Instant::now();
    let mut issues = job.check();
    if let Err(Error::Validation { issues: more }) = output::check_output_dir(&job.output_dir) {
        issues.extend(more);
    }
    for (name, path) in [
        ("dem", &job.dem),
        ("land_cover", &job.land_cover),
        ("origins.path", &job.origins.path),
        ("destinations.path", &job.destinations.path),
    ] {
        if !path.exists() {
            issues.push(format!("{name}: {} does not exist", path.display()));
        }
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }

    let (slots, scenario) = scenario_table(&job.scenario);
    let Landscape {
        grid,
        mut inputs,
        summary,
        plan,
    } = load_landscape(&job.dem, &job.land_cover, &slots, settings, cancelled)?;
    let mut durations_s = BTreeMap::new();
    durations_s.insert("load_inputs", started.elapsed().as_secs_f64());

    let mut warnings = Vec::new();
    let origins = load_facilities(&job.origins, &grid, &mut inputs)?;
    screen_facilities(&origins, job.facility_issues, "origins", &mut warnings)?;
    let destinations = load_facilities(&job.destinations, &grid, &mut inputs)?;
    screen_facilities(
        &destinations,
        job.facility_issues,
        "destinations",
        &mut warnings,
    )?;
    warnings.extend(landscape_warnings(&summary, &job.scenario));

    let started = Instant::now();
    let mut fingerprints = vec![
        sha256_file("dem", &job.dem)?,
        sha256_file("land_cover", &job.land_cover)?,
        sha256_file("origins", &job.origins.path)?,
    ];
    if job.destinations.path != job.origins.path {
        fingerprints.push(sha256_file("destinations", &job.destinations.path)?);
    }
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());

    Ok(PreparedReferral {
        job,
        grid,
        inputs,
        scenario,
        origins,
        destinations,
        summary,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// One output row: an origin → destination pair.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PairRow {
    /// Origin identifier.
    pub origin_id: serde_json::Value,
    /// Destination identifier.
    pub destination_id: serde_json::Value,
    /// Travel time, s; `None` if not reached.
    pub time_s: Option<f64>,
    /// Planar length of the least-time path, m; `None` if not reached.
    pub distance_m: Option<f64>,
}

/// Result counters.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferralSummary {
    /// Origins used.
    pub origins: usize,
    /// Destinations used.
    pub destinations: usize,
    /// Pairs reported.
    pub pairs: usize,
    /// Pairs reached.
    pub reached_pairs: usize,
    /// Searches run.
    pub searches: usize,
    /// Whether searches started from the destinations.
    pub reversed: bool,
    /// Cells settled over all searches.
    pub settled_cells: u64,
}

/// `run.json` of a referral run.
#[derive(Debug, Clone, Serialize)]
pub struct ReferralReport {
    /// Version of this document.
    pub report_version: u32,
    /// Deterministic part.
    pub run: ReferralRecord,
    /// Execution part.
    pub execution: ReferralExecution,
}

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct ReferralRecord {
    /// Job with resolved paths and defaults.
    pub job: ReferralJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// DEM grid.
    pub grid: GridReport,
    /// Input counts.
    pub input_summary: InputSummary,
    /// Result counters.
    pub result: ReferralSummary,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Execution part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct ReferralExecution {
    /// Settings used.
    pub settings: ExecutionSettings,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Worker threads.
    pub workers: usize,
    /// Whether the inputs were shared in memory (else read from scratch).
    pub inputs_in_memory: bool,
    /// Seconds per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
    /// Peak resident memory, bytes, where reported by the platform.
    pub peak_rss_bytes: Option<u64>,
    /// Versions of the engine and libraries.
    pub versions: BTreeMap<&'static str, String>,
}

/// Deviations from AccessMod 5 for referral.
pub fn deviations() -> Vec<Deviation> {
    vec![
        Deviation {
            code: "dem_nodata_barrier",
            message: "DEM NoData cells are barriers (see travel time).",
        },
        Deviation {
            code: "referral_units",
            message: "Times are seconds and distances metres, unrounded. AccessMod 5 rounds \
                      times up (ceiling) in the requested unit and reports distances from \
                      r.path lines.",
        },
        Deviation {
            code: "referral_distance",
            message: "Distance is the planar length of the least-time path between cell \
                      centres, accumulated by the search. AccessMod 5 measures r.path lines \
                      from the facility points with v.net.distance: differences up to one \
                      cell diagonal.",
        },
    ]
}

/// Whether two facilities are the same feature of the same layer.
fn same_feature(
    job: &ReferralJob,
    origins: &LoadedFacilities,
    destinations: &LoadedFacilities,
    o: &FacilityRecord,
    d: &FacilityRecord,
) -> bool {
    job.origins.path == job.destinations.path
        && origins.layer == destinations.layer
        && o.feature_index == d.feature_index
}

/// Compute the pairs and write every output atomically.
pub fn run(
    prepared: PreparedReferral,
    settings: &ExecutionSettings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(PathBuf, ReferralReport)> {
    let PreparedReferral {
        job,
        grid,
        inputs,
        scenario,
        origins,
        destinations,
        summary,
        warnings,
        plan,
        fingerprints,
        mut durations_s,
    } = prepared;
    let staging = Staging::create(&job.output_dir)?;

    let started = Instant::now();
    let frozen = inputs.freeze()?;
    let searches = if job.nearest_only {
        origins.kept.len()
    } else {
        origins.kept.len().min(destinations.kept.len())
    };
    let (workers, budget) = worker_budget(
        &plan,
        frozen.geometry(),
        frozen.in_memory(),
        settings.threads,
        searches,
    );
    let config = ReferralConfig {
        method: job.method(),
        neighbourhood: job.neighbourhood,
        cell: grid.cell,
        limit_s: job.limit_s,
        nearest_only: job.nearest_only,
    };
    let result = referral(
        &frozen,
        &scenario,
        &origins.solver_facilities(),
        &destinations.solver_facilities(),
        &config,
        workers,
        budget,
        &settings.tmp_dir,
        cancelled,
    )?;
    durations_s.insert("solve", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let mut rows = Vec::new();
    let mut nearest_time: Vec<PairRow> = Vec::new();
    let mut nearest_distance: Vec<PairRow> = Vec::new();
    for (oi, o) in origins.kept.iter().enumerate() {
        let mut best_time: Option<(f64, &FacilityRecord, Pair)> = None;
        let mut best_distance: Option<(f64, &FacilityRecord, Pair)> = None;
        for (di, d) in destinations.kept.iter().enumerate() {
            if !job.self_pairs && same_feature(&job, &origins, &destinations, o, d) {
                continue;
            }
            let pair = result.pairs[oi][di];
            if job.nearest_only && pair.is_none() {
                continue;
            }
            rows.push(PairRow {
                origin_id: o.id.clone(),
                destination_id: d.id.clone(),
                time_s: pair.map(|p| p.time_s),
                distance_m: pair.map(|p| p.distance_m),
            });
            if let Some(p) = pair {
                // Strict comparisons: ties keep the first destination.
                if best_time.is_none_or(|(t, _, _)| p.time_s < t) {
                    best_time = Some((p.time_s, d, p));
                }
                if best_distance.is_none_or(|(m, _, _)| p.distance_m < m) {
                    best_distance = Some((p.distance_m, d, p));
                }
            }
        }
        let row = |best: Option<(f64, &FacilityRecord, Pair)>| PairRow {
            origin_id: o.id.clone(),
            destination_id: best.map_or(serde_json::Value::Null, |(_, d, _)| d.id.clone()),
            time_s: best.map(|(_, _, p)| p.time_s),
            distance_m: best.map(|(_, _, p)| p.distance_m),
        };
        nearest_time.push(row(best_time));
        nearest_distance.push(row(best_distance));
    }
    let dir = staging.path();
    write_csv(&dir.join("referral.csv"), &rows)?;
    write_csv(&dir.join("nearest_by_time.csv"), &nearest_time)?;
    if !job.nearest_only {
        write_csv(&dir.join("nearest_by_distance.csv"), &nearest_distance)?;
    }
    output::write_json(&dir.join("job.resolved.json"), &job)?;
    durations_s.insert("write_outputs", started.elapsed().as_secs_f64());

    let mut warnings = warnings;
    if result.reversed {
        warnings.push(Warning {
            code: "referral_reversed",
            message: format!(
                "searches started from the {} destinations (fewer than the {} origins), \
                 with slopes inverted",
                destinations.kept.len(),
                origins.kept.len()
            ),
        });
    }
    let report = ReferralReport {
        report_version: crate::travel_time::RUN_REPORT_VERSION,
        run: ReferralRecord {
            job: job.clone(),
            inputs: fingerprints,
            grid: GridReport {
                cols: grid.cols,
                rows: grid.rows,
                geo_transform: grid.geo_transform,
                crs_wkt: grid.srs.to_wkt().unwrap_or_default(),
            },
            input_summary: summary,
            result: ReferralSummary {
                origins: origins.kept.len(),
                destinations: destinations.kept.len(),
                pairs: rows.len(),
                reached_pairs: rows.iter().filter(|r| r.time_s.is_some()).count(),
                searches: result.searches,
                reversed: result.reversed,
                settled_cells: result.settled,
            },
            warnings,
            deviations: deviations(),
        },
        execution: ReferralExecution {
            settings: settings.clone(),
            plan,
            workers,
            inputs_in_memory: frozen.in_memory(),
            durations_s,
            peak_rss_bytes: resources::peak_rss(),
            versions: versions(),
        },
    };
    output::write_json(&dir.join("run.json"), &report)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let output_dir = staging.commit()?;
    Ok((output_dir, report))
}

/// Workers and per-worker tiles within the memory budget. Shared in-memory
/// inputs are paid once; each worker needs at least the frontier band of
/// state tiles, so fewer workers run when the budget is tight.
fn worker_budget(
    plan: &ResourcePlan,
    geometry: TileGeometry,
    inputs_in_memory: bool,
    threads: usize,
    searches: usize,
) -> (usize, WorkerBudget) {
    let total = geometry.n_tiles() as u64;
    let state_tile = TiledGrid::<SolverCell>::tile_bytes(geometry) as u64;
    let input_tile = TiledGrid::<InputCell>::tile_bytes(geometry) as u64;
    let tiles_x = geometry.cols().div_ceil(geometry.tile()) as u64;
    let tiles_y = geometry.rows().div_ceil(geometry.tile()) as u64;
    let band = (2 * (tiles_x + tiles_y)).min(total);
    let tile_budget = plan
        .memory_budget_bytes
        .saturating_sub(plan.fixed_bytes + plan.queue_reserve_bytes);
    let (per_worker_floor, available) = if inputs_in_memory {
        (
            band * state_tile,
            tile_budget.saturating_sub(total * input_tile),
        )
    } else {
        (band * (state_tile + input_tile), tile_budget)
    };
    let affordable = (available / per_worker_floor.max(1)).max(1) as usize;
    let workers = threads.min(searches.max(1)).min(affordable).max(1);
    let share = available / workers as u64;
    let budget = if inputs_in_memory {
        WorkerBudget {
            state_tiles: (share / state_tile).clamp(1, total) as usize,
            input_tiles: 1,
        }
    } else {
        let tiles = (share / (state_tile + input_tile)).clamp(1, total) as usize;
        WorkerBudget {
            state_tiles: tiles,
            input_tiles: tiles,
        }
    };
    (workers, budget)
}

/// CSV field: identifiers as text, quoted when needed; floats in shortest
/// round-trip form.
fn csv_field(value: &serde_json::Value) -> String {
    let text = match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn write_csv(path: &Path, rows: &[PairRow]) -> Result<()> {
    let mut text = String::from("origin_id,destination_id,time_s,distance_m\n");
    let number = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_default();
    for row in rows {
        let _ = writeln!(
            text,
            "{},{},{},{}",
            csv_field(&row.origin_id),
            csv_field(&row.destination_id),
            number(row.time_s),
            number(row.distance_m)
        );
    }
    std::fs::write(path, text).map_err(Error::file(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_are_quoted_when_needed() {
        assert_eq!(csv_field(&serde_json::json!("HC 1")), "HC 1");
        assert_eq!(csv_field(&serde_json::json!("a,b")), "\"a,b\"");
        assert_eq!(
            csv_field(&serde_json::json!("say \"hi\"")),
            "\"say \"\"hi\"\"\""
        );
        assert_eq!(csv_field(&serde_json::json!(12)), "12");
        assert_eq!(csv_field(&serde_json::Value::Null), "");
    }

    #[test]
    fn minimal_job_parses() {
        let job = ReferralJob::from_json(
            r#"{
              "schema_version": 1, "dem": "d.tif", "land_cover": "l.tif",
              "origins": {"path": "hf.gpkg", "id_field": "code"},
              "destinations": {"path": "hf.gpkg", "id_field": "code", "where": "type = 'H'"},
              "scenario": [{"class": 1, "mode": "walking", "speed_kmh": 5}],
              "method": "anisotropic", "neighbourhood": 16, "output_dir": "out"
            }"#,
        )
        .expect("valid job");
        assert!(job.check().is_empty());
        assert!(!job.nearest_only && !job.self_pairs);
        assert_eq!(job.destinations.filter.as_deref(), Some("type = 'H'"));
    }
}
