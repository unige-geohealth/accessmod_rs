//! Travel-time analysis: validation (`prepare`) and execution (`run`).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::cost::CellMobility;
use accessmod_core::solver::{InputCell, SolveStats, SolverCell, SolverConfig, solve};
use accessmod_core::store::{DEFAULT_TILE, StoreStats, TileGeometry, TiledGrid, tiles_for_budget};
use gdal::Dataset;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::facilities::{FacilityIssue, LoadedFacilities, load_facilities};
use crate::grid::GridSpec;
use crate::inputs::{CHUNK_COLS, InputSummary, STRIP_ROWS, load_inputs, open_land_cover};
use crate::job::{FacilityIssues, MethodName, ScenarioEntry, TravelTimeJob};
use crate::output::{self, Staging};
use crate::resources;

/// Version of the `run.json` document.
pub const RUN_REPORT_VERSION: u32 = 1;

/// Execution settings: they change speed, never results, and are not part
/// of the job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionSettings {
    /// Memory budget, bytes.
    pub memory_bytes: u64,
    /// Threads for GDAL encoding and compression.
    pub threads: usize,
    /// Directory for scratch tiles.
    pub tmp_dir: PathBuf,
}

impl ExecutionSettings {
    /// Defaults from the host: half the available memory, available CPUs,
    /// the system temporary directory.
    pub fn from_host() -> Self {
        Self {
            memory_bytes: resources::default_memory_budget(),
            threads: resources::default_threads(),
            tmp_dir: std::env::temp_dir(),
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

/// Memory and disk plan of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourcePlan {
    /// Memory budget, bytes.
    pub memory_budget_bytes: u64,
    /// Memory reserved for fixed buffers (GDAL cache, output strips), bytes.
    pub fixed_bytes: u64,
    /// Memory reserved for the priority queue, bytes.
    pub queue_reserve_bytes: u64,
    /// GDAL block cache (`GDAL_CACHEMAX`), bytes.
    pub gdal_cache_bytes: u64,
    /// Tile side, cells.
    pub tile: usize,
    /// Tiles per grid.
    pub total_tiles: usize,
    /// Resident tiles per grid.
    pub resident_tiles: usize,
    /// Memory to keep every tile resident (no scratch I/O), bytes.
    pub in_memory_bytes: u64,
    /// Smallest budget accepted, bytes.
    pub minimum_bytes: u64,
    /// Scratch disk needed in the worst case, bytes (0 when every tile is
    /// resident).
    pub scratch_bytes: u64,
    /// Free space where scratch files go, bytes, if known.
    pub free_disk_bytes: Option<u64>,
}

impl ResourcePlan {
    /// Plan for a `rows × cols` grid under `settings`, or the reason it does
    /// not fit (both estimates included).
    pub fn new(rows: usize, cols: usize, settings: &ExecutionSettings) -> Result<Self> {
        let geometry = TileGeometry::new(rows, cols, DEFAULT_TILE)?;
        let per_tile = (TiledGrid::<InputCell>::tile_bytes(geometry)
            + TiledGrid::<SolverCell>::tile_bytes(geometry)) as u64;
        let total_tiles = geometry.n_tiles();
        let budget = settings.memory_bytes;
        let gdal_cache_bytes = (budget / 8).clamp(16 << 20, 512 << 20);
        // Output blocks (4 rasters) and input read windows (DEM f64, land
        // cover i64, classes); both bounded whatever the grid width.
        let buffer_bytes = (256 * 256 * (8 + 4 + 8 + 1)
            + 256 * std::mem::size_of::<SolverCell>()
            + STRIP_ROWS * CHUNK_COLS * (8 + 8)
            + CHUNK_COLS * 16) as u64;
        let fixed_bytes = gdal_cache_bytes + buffer_bytes;
        let queue_reserve_bytes = budget / 10;
        // The solver needs at least the band of tiles around its frontier;
        // two tile rows and two tile columns across the grid is the floor.
        let tiles_x = cols.div_ceil(DEFAULT_TILE);
        let tiles_y = rows.div_ceil(DEFAULT_TILE);
        let band_tiles = (2 * (tiles_x + tiles_y)).min(total_tiles) as u64;
        // The queue reserve is 10 % of the budget B, so tiles get
        // 0.9·B − fixed: B must be at least (fixed + tiles) / 0.9.
        let with_reserve = |tile_bytes: u64| (fixed_bytes + tile_bytes).div_ceil(9) * 10;
        let minimum_bytes = with_reserve(band_tiles * per_tile);
        let in_memory_bytes = with_reserve(total_tiles as u64 * per_tile);
        let tile_budget = budget.saturating_sub(fixed_bytes + queue_reserve_bytes);
        let resident_tiles = tiles_for_budget(tile_budget, &[per_tile as usize]).min(total_tiles);
        let scratch_bytes = if resident_tiles >= total_tiles {
            0
        } else {
            total_tiles as u64 * per_tile
        };
        let free_disk_bytes = resources::free_disk(&settings.tmp_dir);
        let plan = Self {
            memory_budget_bytes: budget,
            fixed_bytes,
            queue_reserve_bytes,
            gdal_cache_bytes,
            tile: DEFAULT_TILE,
            total_tiles,
            resident_tiles,
            in_memory_bytes,
            minimum_bytes,
            scratch_bytes,
            free_disk_bytes,
        };
        let mut issues = Vec::new();
        if budget < minimum_bytes {
            issues.push(format!(
                "memory budget {} is below the minimum {} for a {rows} x {cols} grid \
                 ({} keeps everything in memory); raise --memory",
                mib(budget),
                mib(minimum_bytes),
                mib(in_memory_bytes)
            ));
        }
        if let Some(free) = free_disk_bytes
            && free < scratch_bytes
        {
            issues.push(format!(
                "scratch directory {} has {} free, the run may need {}; use --tmp-dir \
                 or raise --memory (in memory: {})",
                settings.tmp_dir.display(),
                mib(free),
                mib(scratch_bytes),
                mib(in_memory_bytes)
            ));
        }
        if issues.is_empty() {
            Ok(plan)
        } else {
            Err(Error::Validation { issues })
        }
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.0} MiB", bytes as f64 / (1u64 << 20) as f64)
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

pub(crate) fn sha256_file(role: &'static str, path: &Path) -> Result<Fingerprint> {
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
    if let Err(Error::Validation { issues: more }) = output::check_output_dir(&job.output_dir) {
        issues.extend(more);
    }
    for (name, path) in [
        ("dem", &job.dem),
        ("land_cover", &job.land_cover),
        ("facilities.path", &job.facilities.path),
    ] {
        if !path.exists() {
            issues.push(format!("{name}: {} does not exist", path.display()));
        }
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }

    let (slots, scenario) = job.scenario_table();
    let Landscape {
        grid,
        mut inputs,
        summary,
        plan,
    } = load_landscape(&job.dem, &job.land_cover, &slots, settings, cancelled)?;
    durations_s.insert("load_inputs", started.elapsed().as_secs_f64());

    let facilities = load_facilities(&job.facilities, &grid, &mut inputs)?;
    let mut warnings = Vec::new();
    screen_facilities(
        &facilities,
        job.facility_issues,
        "facilities",
        &mut warnings,
    )?;
    warnings.extend(landscape_warnings(&summary, &job.scenario));

    let fingerprint_started = Instant::now();
    let fingerprints = vec![
        sha256_file("dem", &job.dem)?,
        sha256_file("land_cover", &job.land_cover)?,
        sha256_file("facilities", &job.facilities.path)?,
    ];
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

/// Open the DEM and the land cover, check them, plan resources and load the
/// solver inputs.
pub(crate) fn load_landscape(
    dem_path: &Path,
    land_cover_path: &Path,
    slots: &BTreeMap<i64, u16>,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<Landscape> {
    let dem = Dataset::open(dem_path).map_err(Error::gdal(dem_path))?;
    let grid = GridSpec::from_dem(&dem, dem_path).map_err(|issues| Error::Validation { issues })?;
    let (land_cover, alignment) = open_land_cover(land_cover_path, &grid)?;
    let plan = ResourcePlan::new(grid.rows, grid.cols, settings)?;
    gdal::config::set_config_option("GDAL_CACHEMAX", &plan.gdal_cache_bytes.to_string())
        .map_err(Error::gdal(dem_path))?;
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

/// Execution part of `run.json`: varies with host and settings.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionReport {
    /// Settings used.
    pub settings: ExecutionSettings,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Seconds per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
    /// Cache counters of the input grid.
    pub input_store: StoreStats,
    /// Cache counters of the state grid.
    pub state_store: StoreStats,
    /// Peak resident memory, bytes, where reported by the platform.
    pub peak_rss_bytes: Option<u64>,
    /// Versions of the engine and libraries.
    pub versions: BTreeMap<&'static str, String>,
}

/// `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    /// Version of this document.
    pub report_version: u32,
    /// Deterministic part.
    pub run: RunRecord,
    /// Execution part.
    pub execution: ExecutionReport,
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
    let result = summarise(&mut state, &prepared.grid)?;
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
    output::write_json(&staging.path().join("job.resolved.json"), job)?;

    let mut warnings = prepared.warnings.clone();
    for (ordinal, kept) in &stats.shared_cells {
        warnings.push(Warning {
            code: "facilities_share_cell",
            message: format!(
                "facility ordinal {ordinal} shares its cell with ordinal {kept}, which is kept"
            ),
        });
    }
    let report = RunReport {
        report_version: RUN_REPORT_VERSION,
        run: RunRecord {
            job: job.clone(),
            inputs: prepared.fingerprints.clone(),
            grid: GridReport {
                cols: prepared.grid.cols,
                rows: prepared.grid.rows,
                geo_transform: prepared.grid.geo_transform,
                crs_wkt: prepared.grid.srs.to_wkt().unwrap_or_default(),
            },
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
        execution: ExecutionReport {
            settings: settings.clone(),
            plan: prepared.plan.clone(),
            durations_s: prepared.durations_s.clone(),
            input_store: prepared.inputs.stats(),
            state_store: state.stats(),
            peak_rss_bytes: resources::peak_rss(),
            versions: versions(),
        },
    };
    output::write_json(&staging.path().join("run.json"), &report)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let output_dir = staging.commit()?;
    Ok((output_dir, report))
}

fn summarise(state: &mut TiledGrid<SolverCell>, grid: &GridSpec) -> Result<ResultSummary> {
    let mut summary = ResultSummary::default();
    let mut row = vec![SolverCell::UNREACHED; grid.cols];
    for r in 0..grid.rows {
        state.read_row(r, 0, &mut row)?;
        for cell in row.iter().filter(|c| c.is_reached()) {
            summary.reached_cells += 1;
            summary.max_travel_time_s = summary.max_travel_time_s.max(cell.time_s);
            summary.max_path_distance_m = summary.max_path_distance_m.max(cell.distance_m);
        }
    }
    Ok(summary)
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
        let probe = ResourcePlan::new(rows, cols, &settings(64 << 30)).expect("roomy plan");
        assert_eq!(probe.resident_tiles, probe.total_tiles);
        assert_eq!(probe.scratch_bytes, 0);

        // The GDAL cache scales with the budget, so the minimum does too:
        // any budget at or above its own minimum keeps the frontier band.
        let band = 2 * (cols.div_ceil(DEFAULT_TILE) + rows.div_ceil(DEFAULT_TILE));
        let mut budget = probe.minimum_bytes;
        for _ in 0..4 {
            let plan = ResourcePlan::new(rows, cols, &settings(budget)).expect("at minimum");
            assert!(
                plan.resident_tiles >= band,
                "{} < {band}",
                plan.resident_tiles
            );
            assert!(plan.scratch_bytes > 0);
            budget = plan.minimum_bytes;
        }

        let err = ResourcePlan::new(rows, cols, &settings(32 << 20)).expect_err("too small");
        assert_eq!(err.kind(), crate::error::ErrorKind::Validation);
        assert!(err.to_string().contains("--memory"), "{err}");
    }
}
