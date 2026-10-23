//! Travel-time analysis: validation (`prepare`) and execution (`run`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::cost::CellMobility;
use accessmod_core::solver::{InputCell, SolveStats, SolverCell, SolverConfig, solve};
use accessmod_core::store::{DEFAULT_TILE, StoreStats, TileGeometry, TiledGrid};
use serde::Serialize;

use crate::analysis::{Analysis, Cancelled};
use crate::error::{Error, Result};
use crate::facilities::{FacilityIssue, LoadedFacilities, load_facilities};
use crate::grid::GridSpec;
use crate::inputs::{InputSummary, load_inputs};
use crate::job::{FacilityIssues, MethodName, ScenarioEntry, TravelTimeJob, check_files};
use crate::output::{self, Staging};
use crate::raster::{CHUNK_COLS, STRIP_ROWS, open_class_raster};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};

/// Memory and disk plan of a run on the solver: input and state grids,
/// `extra_per_tile` bytes per tile of further grids and `extra_bytes` of
/// further buffers (0 for travel time and referral), a priority queue, output
/// blocks and input read windows.
pub fn solver_plan(
    rows: usize,
    cols: usize,
    settings: &ExecutionSettings,
    extra_per_tile: u64,
    extra_bytes: u64,
) -> Result<ResourcePlan> {
    let geometry = TileGeometry::new(rows, cols, DEFAULT_TILE)?;
    let per_tile = (TiledGrid::<InputCell>::tile_bytes(geometry)
        + TiledGrid::<SolverCell>::tile_bytes(geometry)) as u64
        + extra_per_tile;
    // Output blocks (4 rasters) and input read windows (DEM f64, land
    // cover i64, classes); both bounded whatever the grid width.
    let buffer_bytes = (256 * 256 * (8 + 4 + 8 + 1)
        + 256 * std::mem::size_of::<SolverCell>()
        + STRIP_ROWS * CHUNK_COLS * (8 + 8)
        + CHUNK_COLS * 16) as u64
        + extra_bytes;
    ResourcePlan::for_tiles(rows, cols, settings, per_tile, buffer_bytes, 1)
}

/// Inputs loaded and validated, ready to solve.
#[derive(Debug)]
pub struct Prepared {
    /// Job with resolved paths.
    pub job: TravelTimeJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Solver inputs.
    pub inputs: TiledGrid<InputCell>,
    /// Scenario slots.
    pub scenario: Vec<CellMobility>,
    /// Facilities used and rejected.
    pub facilities: LoadedFacilities,
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

fn facility_issue_text(issue: &FacilityIssue) -> String {
    let id = issue
        .id
        .as_ref()
        .map_or_else(|| "no id".to_owned(), |id| format!("id {id}"));
    format!(
        "facility {} ({id}): {}",
        issue.fid.map_or_else(
            || format!("#{}", issue.feature_index + 1),
            |fid| format!("FID {fid}")
        ),
        issue.reason
    )
}

/// Validate the job against its inputs and load them: everything short of
/// solving. Fails with every validation problem found.
pub fn prepare(
    job: TravelTimeJob,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<Prepared> {
    let mut durations_s = BTreeMap::new();
    let started = Instant::now();
    let mut issues = job.check();
    issues.extend(check_files(
        &job.output_dir,
        [
            ("dem".to_owned(), job.dem.as_path()),
            ("land_cover".to_owned(), job.land_cover.as_path()),
            ("facilities.path".to_owned(), job.facilities.path.as_path()),
        ],
    ));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }

    let (slots, scenario) = job.scenario_table();
    let Landscape {
        grid,
        mut inputs,
        summary,
        plan,
    } = load_landscape(&job.dem, &job.land_cover, &slots, settings, 0, 0, cancelled)?;
    durations_s.insert("load_inputs", started.elapsed().as_secs_f64());

    let facilities = load_facilities(&job.facilities, &grid, &mut inputs, cancelled)?;
    let mut warnings = Vec::new();
    screen_facilities(
        &facilities,
        job.facility_issues,
        "facilities",
        &mut warnings,
    )?;
    warnings.extend(landscape_warnings(&summary, &job.scenario));

    let fingerprint_started = Instant::now();
    let fingerprints = fingerprints(
        &[
            ("dem", job.dem.as_path()),
            ("land_cover", job.land_cover.as_path()),
            ("facilities", job.facilities.path.as_path()),
        ],
        cancelled,
    )?;
    durations_s.insert("fingerprints", fingerprint_started.elapsed().as_secs_f64());

    Ok(Prepared {
        job,
        grid,
        inputs,
        scenario,
        facilities,
        summary,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// DEM grid and solver inputs, loaded and validated.
#[derive(Debug)]
pub(crate) struct Landscape {
    pub grid: GridSpec,
    pub inputs: TiledGrid<InputCell>,
    pub summary: InputSummary,
    pub plan: ResourcePlan,
}

/// Open the DEM and the land cover, check them, plan resources (see
/// [`solver_plan`] for `extra_per_tile` and `extra_bytes`) and load the solver
/// inputs.
#[allow(clippy::too_many_arguments)] // reason: one crate-private loader, explicit inputs
pub(crate) fn load_landscape(
    dem_path: &Path,
    land_cover_path: &Path,
    slots: &BTreeMap<i64, u16>,
    settings: &ExecutionSettings,
    extra_per_tile: u64,
    extra_bytes: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Landscape> {
    let (dem, grid) = GridSpec::open_dem(dem_path)?;
    let (land_cover, alignment) = open_class_raster(land_cover_path, &grid, "land cover")?;
    let plan = solver_plan(grid.rows, grid.cols, settings, extra_per_tile, extra_bytes)?;
    plan.configure_gdal()?;
    let geometry = TileGeometry::new(grid.rows, grid.cols, plan.tile)?;
    let mut inputs = TiledGrid::new(
        geometry,
        InputCell::NODATA,
        plan.resident_tiles,
        &settings.tmp_dir,
        "inputs",
    )?;
    let summary = load_inputs(
        &dem,
        dem_path,
        &land_cover,
        land_cover_path,
        alignment,
        &grid,
        slots,
        &mut inputs,
        cancelled,
    )?;
    Ok(Landscape {
        grid,
        inputs,
        summary,
        plan,
    })
}

/// Fail on unusable facilities, or turn them into warnings, per `policy`;
/// fail when none is usable. `role` names the layer in messages.
pub(crate) fn screen_facilities(
    facilities: &LoadedFacilities,
    policy: FacilityIssues,
    role: &str,
    warnings: &mut Vec<Warning>,
) -> Result<()> {
    if !facilities.issues.is_empty() {
        let texts: Vec<String> = facilities
            .issues
            .iter()
            .map(|issue| format!("{role}: {}", facility_issue_text(issue)))
            .collect();
        match policy {
            FacilityIssues::Error => {
                let mut issues = texts;
                issues.push(
                    "fix these facilities or set \"facility_issues\": \"skip\" to ignore them"
                        .to_owned(),
                );
                return Err(Error::Validation { issues });
            }
            FacilityIssues::Skip => warnings.extend(texts.into_iter().map(|message| Warning {
                code: "facility_skipped",
                message,
            })),
        }
    }
    if facilities.kept.is_empty() {
        return Err(Error::invalid(format!("{role}: no usable facility")));
    }
    if facilities.reprojected {
        warnings.push(Warning {
            code: "facilities_reprojected",
            message: format!(
                "{role}: points were reprojected from layer {:?} CRS to the DEM CRS",
                facilities.layer
            ),
        });
    }
    Ok(())
}

/// Warnings about land-cover classes and the scenario.
pub(crate) fn landscape_warnings(
    summary: &InputSummary,
    scenario: &[ScenarioEntry],
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    if !summary.unmapped_classes.is_empty() || summary.unmapped_other_cells > 0 {
        let listed: Vec<String> = summary
            .unmapped_classes
            .iter()
            .map(|(class, cells)| format!("{class} ({cells} cells)"))
            .collect();
        let mut message = format!(
            "land-cover classes without scenario are barriers: {}",
            listed.join(", ")
        );
        if summary.unmapped_other_cells > 0 {
            message.push_str(&format!(
                " and {} cells of further classes",
                summary.unmapped_other_cells
            ));
        }
        warnings.push(Warning {
            code: "classes_without_scenario",
            message,
        });
    }
    for entry in scenario {
        if !summary.class_cells.contains_key(&entry.class) {
            warnings.push(Warning {
                code: "scenario_class_absent",
                message: format!(
                    "scenario class {} does not occur in the land cover",
                    entry.class
                ),
            });
        }
    }
    warnings
}

/// Deviations from AccessMod 5 relevant to a job.
pub fn deviations(job: &TravelTimeJob) -> Vec<Deviation> {
    let mut list = vec![
        Deviation {
            code: "dem_nodata_barrier",
            message: "DEM NoData cells are barriers. AccessMod 5 still assigns a motorized \
                      travel time to such cells (without expanding from them).",
        },
        Deviation {
            code: "exact_limit",
            message: "The limit is applied during the search. r.walk overshoots by one step \
                      and AccessMod 5 removes the overshoot afterwards with the same \
                      inclusive rule.",
        },
        Deviation {
            code: "tie_break",
            message: "Equal travel times are resolved by path distance, then facility order, \
                      then cell index. AccessMod 5 keeps the first value found.",
        },
    ];
    if job.method == MethodName::Isotropic {
        list.push(Deviation {
            code: "isotropic_pace_average",
            message: "Isotropic costs are computed in f64 from the scenario speeds. \
                      AccessMod 5 runs r.cost on a Float32 friction map, so its times \
                      differ by up to 2^-24 relative.",
        });
    }
    list
}

/// Facility table written next to `nearest_facility.tif`.
#[derive(Debug, Clone, Serialize)]
pub struct FacilityTable<'a> {
    /// Layer read.
    pub layer: &'a str,
    /// Facilities used; `ordinal` is the value in `nearest_facility.tif`.
    pub facilities: &'a [crate::facilities::FacilityRecord],
    /// Facilities not used, with the reason.
    pub skipped: &'a [FacilityIssue],
    /// Facilities sharing a cell with an earlier one: (ordinal, ordinal kept).
    pub shared_cells: &'a [(u32, u32)],
}

/// Result counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ResultSummary {
    /// Cells reached by a facility.
    pub reached_cells: u64,
    /// Largest travel time, s.
    pub max_travel_time_s: f64,
    /// Largest path distance, m.
    pub max_path_distance_m: f64,
}

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    /// Job with resolved paths and defaults.
    pub job: TravelTimeJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// DEM grid.
    pub grid: GridReport,
    /// Input counts.
    pub input_summary: InputSummary,
    /// Facilities used.
    pub facilities_used: usize,
    /// Facilities skipped.
    pub facilities_skipped: usize,
    /// Result counters.
    pub result: ResultSummary,
    /// Solver counters.
    pub solver: SolverReport,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Solver counters that do not depend on execution settings.
#[derive(Debug, Clone, Serialize)]
pub struct SolverReport {
    /// Cells settled.
    pub settled: u64,
    /// Heap pops.
    pub pops: u64,
    /// Peak heap size.
    pub heap_peak: usize,
    /// Facilities sharing a cell: (ordinal, ordinal kept).
    pub shared_cells: Vec<(u32, u32)>,
}

/// Execution details of a travel-time run.
#[derive(Debug, Clone, Serialize)]
pub struct SolverStores {
    /// Cache counters of the input grid.
    pub input_store: StoreStats,
    /// Cache counters of the state grid.
    pub state_store: StoreStats,
}

/// `run.json` of a travel-time run.
pub type RunReport = Report<RunRecord, SolverStores>;

/// Solve a prepared job and write every output atomically.
pub fn run(
    mut prepared: Prepared,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<(PathBuf, RunReport)> {
    let job = &prepared.job;
    let staging = Staging::create(&job.output_dir)?;
    let geometry = prepared.inputs.geometry();
    let mut state = TiledGrid::new(
        geometry,
        SolverCell::UNREACHED,
        prepared.plan.resident_tiles,
        &settings.tmp_dir,
        "state",
    )?;
    let config = SolverConfig {
        method: job.method(),
        direction: job.direction,
        neighbourhood: job.neighbourhood,
        cell: prepared.grid.cell,
        limit_s: job.limit_s,
    };
    let started = Instant::now();
    let stats: SolveStats = solve(
        &mut prepared.inputs,
        &prepared.scenario,
        &prepared.facilities.solver_facilities(),
        &config,
        &mut state,
        cancelled,
    )?;
    prepared
        .durations_s
        .insert("solve", started.elapsed().as_secs_f64());

    let started = Instant::now();
    output::write_rasters(
        staging.path(),
        &prepared.grid,
        &mut state,
        job.outputs.direction,
        settings.threads,
        cancelled,
    )?;
    let result = summarise(&mut state, cancelled)?;
    prepared
        .durations_s
        .insert("write_outputs", started.elapsed().as_secs_f64());

    let table = FacilityTable {
        layer: &prepared.facilities.layer,
        facilities: &prepared.facilities.kept,
        skipped: &prepared.facilities.issues,
        shared_cells: &stats.shared_cells,
    };
    output::write_json(&staging.path().join("nearest_facility.json"), &table)?;

    let mut warnings = prepared.warnings.clone();
    for (ordinal, kept) in &stats.shared_cells {
        warnings.push(Warning {
            code: "facilities_share_cell",
            message: format!(
                "facility ordinal {ordinal} shares its cell with ordinal {kept}, which is kept"
            ),
        });
    }
    let stores = SolverStores {
        input_store: prepared.inputs.stats(),
        state_store: state.stats(),
    };
    let report = Report::new(
        RunRecord {
            job: job.clone(),
            inputs: prepared.fingerprints.clone(),
            grid: GridReport::of(&prepared.grid),
            input_summary: prepared.summary.clone(),
            facilities_used: prepared.facilities.kept.len(),
            facilities_skipped: prepared.facilities.issues.len(),
            result,
            solver: SolverReport {
                settled: stats.settled,
                pops: stats.pops,
                heap_peak: stats.heap_peak,
                shared_cells: stats.shared_cells.clone(),
            },
            warnings,
            deviations: deviations(job),
        },
        Execution::now(
            settings,
            prepared.plan.clone(),
            prepared.durations_s.clone(),
            stores,
        ),
    );
    let output_dir = output::finish(staging, job, &report, cancelled)?;
    Ok((output_dir, report))
}

fn summarise(
    state: &mut TiledGrid<SolverCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<ResultSummary> {
    let mut summary = ResultSummary::default();
    state.for_each_row_piece(output::BLOCK, |_, col, cells| {
        if col == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        for cell in cells.iter().filter(|c| c.is_reached()) {
            summary.reached_cells += 1;
            summary.max_travel_time_s = summary.max_travel_time_s.max(cell.time_s);
            summary.max_path_distance_m = summary.max_path_distance_m.max(cell.distance_m);
        }
        Ok(())
    })?;
    Ok(summary)
}

/// Travel-time analysis.
#[derive(Debug)]
pub struct TravelTime;

impl Analysis for TravelTime {
    const NAME: &'static str = "travel-time";
    type Job = TravelTimeJob;
    type Prepared = Prepared;
    type Report = RunReport;

    fn prepare(
        job: TravelTimeJob,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<Prepared> {
        prepare(job, settings, cancelled)
    }

    fn run(
        prepared: Prepared,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<(PathBuf, RunReport)> {
        run(prepared, settings, cancelled)
    }

    fn warnings(prepared: &Prepared) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &Prepared) -> String {
        format!(
            "travel time: {} x {} grid, {} facilities, {} passable cells ({} of {} tiles resident)",
            prepared.grid.cols,
            prepared.grid.rows,
            prepared.facilities.kept.len(),
            prepared.summary.passable,
            prepared.plan.resident_tiles,
            prepared.plan.total_tiles
        )
    }

    fn validation(prepared: &Prepared) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "grid": {"cols": prepared.grid.cols, "rows": prepared.grid.rows,
                     "geo_transform": prepared.grid.geo_transform},
            "input_summary": prepared.summary,
            "facilities_used": prepared.facilities.kept.len(),
            "facilities_skipped": prepared.facilities.issues,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &RunReport) -> String {
        format!(
            "{} cells reached, max {:.0} s",
            report.run.result.reached_cells, report.run.result.max_travel_time_s
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(memory_bytes: u64) -> ExecutionSettings {
        ExecutionSettings {
            memory_bytes,
            threads: 1,
            tmp_dir: std::env::temp_dir(),
        }
    }

    #[test]
    fn minimum_budget_leaves_the_frontier_band() {
        let (rows, cols) = (5000, 7000);
        let probe = solver_plan(rows, cols, &settings(64 << 30), 0, 0).expect("roomy plan");
        assert_eq!(probe.resident_tiles, probe.total_tiles);
        assert_eq!(probe.scratch_bytes, 0);

        // The GDAL cache scales with the budget, so the minimum does too:
        // any budget at or above its own minimum keeps the frontier band.
        let band = 2 * (cols.div_ceil(DEFAULT_TILE) + rows.div_ceil(DEFAULT_TILE));
        let mut budget = probe.minimum_bytes;
        for _ in 0..4 {
            let plan = solver_plan(rows, cols, &settings(budget), 0, 0).expect("at minimum");
            assert!(
                plan.resident_tiles >= band,
                "{} < {band}",
                plan.resident_tiles
            );
            assert!(plan.scratch_bytes > 0);
            budget = plan.minimum_bytes;
        }

        let err = solver_plan(rows, cols, &settings(32 << 20), 0, 0).expect_err("too small");
        assert_eq!(err.kind(), crate::error::ErrorKind::Validation);
        assert!(err.to_string().contains("--memory"), "{err}");
    }
}
