//! Referral analysis: travel time and path distance between facilities
//! (`docs/referral.md`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::cost::{CellMobility, Method, Neighbourhood};
use accessmod_core::mobility::MobilityModel;
use accessmod_core::referral::{Pair, ReferralConfig, SearchFrom, WorkerBudget, referral};
use accessmod_core::solver::{InputCell, SolverCell};
use accessmod_core::store::{TileGeometry, TiledGrid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Cancelled};
use crate::error::{Error, Result};
use crate::facilities::{FacilityRecord, LoadedFacilities, load_facilities};
use crate::grid::GridSpec;
use crate::inputs::InputSummary;
use crate::job::{
    FacilityIssues, FacilitySource, JobFile, MethodName, ScenarioEntry, absolutize, check_common,
    check_files, scenario_table,
};
use crate::output::{self, CsvWriter, Staging, csv_field};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::travel_time::{Landscape, landscape_warnings, load_landscape, screen_facilities};

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
    /// Keep pairs from a facility to itself (same FID of the same layer);
    /// AccessMod 5 drops them.
    #[serde(default)]
    pub self_pairs: bool,
    /// Which side searches start from: `auto` (the smaller side, equal to
    /// forward searches up to floating-point rounding) or `origins` (exact
    /// forward searches).
    #[serde(default)]
    pub search_from: SearchFrom,
    /// What to do with facilities outside the grid or on a barrier.
    #[serde(default)]
    pub facility_issues: FacilityIssues,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

impl JobFile for ReferralJob {
    fn resolve_paths(mut self, base: &Path) -> Self {
        for path in [
            &mut self.dem,
            &mut self.land_cover,
            &mut self.origins.path,
            &mut self.destinations.path,
            &mut self.output_dir,
        ] {
            absolutize(path, base);
        }
        self
    }
}

impl ReferralJob {
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
    issues.extend(check_files(
        &job.output_dir,
        [
            ("dem".to_owned(), job.dem.as_path()),
            ("land_cover".to_owned(), job.land_cover.as_path()),
            ("origins.path".to_owned(), job.origins.path.as_path()),
            (
                "destinations.path".to_owned(),
                job.destinations.path.as_path(),
            ),
        ],
    ));
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
    // The pair matrix is held in memory until written: refuse jobs whose
    // matrix would take more than half the budget.
    let matrix_bytes = (origins.kept.len() as u64)
        .saturating_mul(destinations.kept.len() as u64)
        .saturating_mul(std::mem::size_of::<Option<Pair>>() as u64);
    if matrix_bytes > settings.memory_bytes / 2 {
        return Err(Error::invalid(format!(
            "{} origins x {} destinations need {} MiB of results, more than half the \
             memory budget; raise --memory, use nearest_only, or split the origins with \
             a filter",
            origins.kept.len(),
            destinations.kept.len(),
            matrix_bytes >> 20
        )));
    }
    warnings.extend(landscape_warnings(&summary, &job.scenario));

    let started = Instant::now();
    let mut files = vec![
        ("dem", job.dem.as_path()),
        ("land_cover", job.land_cover.as_path()),
        ("origins", job.origins.path.as_path()),
    ];
    if job.destinations.path != job.origins.path {
        files.push(("destinations", job.destinations.path.as_path()));
    }
    let fingerprints = fingerprints(&files, cancelled)?;
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
pub type ReferralReport = Report<ReferralRecord, ReferralWorkers>;

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

/// Execution details of a referral run.
#[derive(Debug, Clone, Serialize)]
pub struct ReferralWorkers {
    /// Worker threads.
    pub workers: usize,
    /// Whether the inputs were shared in memory (else read from scratch).
    pub inputs_in_memory: bool,
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
        // Only real FIDs identify a feature across two filtered reads.
        && o.fid.is_some()
        && o.fid == d.fid
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
        search_from: job.search_from,
    };
    let excluded = |o: usize, d: usize| {
        !job.self_pairs
            && same_feature(
                &job,
                &origins,
                &destinations,
                &origins.kept[o],
                &destinations.kept[d],
            )
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
        &excluded,
        cancelled,
    )?;
    durations_s.insert("solve", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let dir = staging.path();
    let pairs_csv = |name: &str| {
        CsvWriter::create(
            &dir.join(name),
            "origin_id,destination_id,time_s,distance_m",
        )
    };
    let mut all = pairs_csv("referral.csv")?;
    let mut by_time = pairs_csv("nearest_by_time.csv")?;
    let mut by_distance = if job.nearest_only {
        None
    } else {
        Some(pairs_csv("nearest_by_distance.csv")?)
    };
    let (mut pairs, mut reached_pairs) = (0usize, 0usize);
    for (oi, o) in origins.kept.iter().enumerate() {
        let row = &result.pairs[oi];
        let eligible = |di: &usize| !excluded(oi, *di);
        for di in (0..destinations.kept.len()).filter(eligible) {
            let pair = row[di];
            if job.nearest_only && pair.is_none() {
                continue;
            }
            all.record(&pair_fields(&o.id, Some(&destinations.kept[di].id), pair))?;
            pairs += 1;
            reached_pairs += usize::from(pair.is_some());
        }
        // Nearest by time and by distance: every destination at the minimum
        // (ties kept, in destination order, as AccessMod 5); an origin
        // reaching none gets one row without destination.
        let reached: Vec<(usize, Pair)> = (0..destinations.kept.len())
            .filter(eligible)
            .filter_map(|di| row[di].map(|p| (di, p)))
            .collect();
        let nearest = |key: fn(&Pair) -> f64, out: &mut CsvWriter| -> Result<()> {
            let best = reached
                .iter()
                .map(|(_, p)| key(p))
                .fold(f64::INFINITY, f64::min);
            if reached.is_empty() {
                return out.record(&pair_fields(&o.id, None, None));
            }
            for (di, p) in reached.iter().filter(|(_, p)| key(p) == best) {
                out.record(&pair_fields(
                    &o.id,
                    Some(&destinations.kept[*di].id),
                    Some(*p),
                ))?;
            }
            Ok(())
        };
        nearest(|p| p.time_s, &mut by_time)?;
        if let Some(out) = by_distance.as_mut() {
            nearest(|p| p.distance_m, out)?;
        }
    }
    all.finish()?;
    by_time.finish()?;
    if let Some(out) = by_distance {
        out.finish()?;
    }
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
    let details = ReferralWorkers {
        workers,
        inputs_in_memory: frozen.in_memory(),
    };
    let report = Report::new(
        ReferralRecord {
            job,
            inputs: fingerprints,
            grid: GridReport::of(&grid),
            input_summary: summary,
            result: ReferralSummary {
                origins: origins.kept.len(),
                destinations: destinations.kept.len(),
                pairs,
                reached_pairs,
                searches: result.searches,
                reversed: result.reversed,
                settled_cells: result.settled,
            },
            warnings,
            deviations: deviations(),
        },
        Execution::now(settings, plan, durations_s, details),
    );
    let output_dir = output::finish(staging, &report.run.job, &report, cancelled)?;
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
    // Each worker has its own priority queue: reserve room for a frontier
    // of 16 entries (32 bytes) per boundary cell of the grid.
    let queue = 16 * 32 * 2 * (geometry.rows() + geometry.cols()) as u64;
    let (per_worker_floor, available) = if inputs_in_memory {
        (
            band * state_tile + queue,
            tile_budget.saturating_sub(total * input_tile),
        )
    } else {
        (band * (state_tile + input_tile) + queue, tile_budget)
    };
    let affordable = (available / per_worker_floor.max(1)).max(1) as usize;
    let workers = threads.min(searches.max(1)).min(affordable).max(1);
    let share = (available / workers as u64).saturating_sub(queue);
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

/// CSV fields of a pair: `origin_id,destination_id,time_s,distance_m`, empty
/// for a missing destination or an unreached pair.
fn pair_fields(
    origin: &serde_json::Value,
    destination: Option<&serde_json::Value>,
    pair: Option<Pair>,
) -> [String; 4] {
    let number = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_default();
    [
        csv_field(origin),
        destination.map(csv_field).unwrap_or_default(),
        number(pair.map(|p| p.time_s)),
        number(pair.map(|p| p.distance_m)),
    ]
}

/// Referral analysis.
#[derive(Debug)]
pub struct Referral;

impl Analysis for Referral {
    type Job = ReferralJob;
    type Prepared = PreparedReferral;
    type Report = ReferralReport;

    fn prepare(
        job: ReferralJob,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<PreparedReferral> {
        prepare(job, settings, cancelled)
    }

    fn run(
        prepared: PreparedReferral,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<(PathBuf, ReferralReport)> {
        run(prepared, settings, cancelled)
    }

    fn warnings(prepared: &PreparedReferral) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &PreparedReferral) -> String {
        format!(
            "referral: {} origins x {} destinations on a {} x {} grid",
            prepared.origins.kept.len(),
            prepared.destinations.kept.len(),
            prepared.grid.cols,
            prepared.grid.rows
        )
    }

    fn validation(prepared: &PreparedReferral) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "origins_used": prepared.origins.kept.len(),
            "origins_skipped": prepared.origins.issues,
            "destinations_used": prepared.destinations.kept.len(),
            "destinations_skipped": prepared.destinations.issues,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &ReferralReport) -> String {
        let r = &report.run.result;
        format!(
            "{} pairs ({} reached) from {} searches{}",
            r.pairs,
            r.reached_pairs,
            r.searches,
            if r.reversed {
                ", started from the destinations"
            } else {
                ""
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
