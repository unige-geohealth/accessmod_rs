//! Geographic coverage (schema version 1): facilities take, one after the
//! other, the population they can reach up to their capacity, as AccessMod 5
//! does (`tools/R/amAnalysisCapacity.R`, catchment rules in
//! `accessmod_core::capacity`).
//!
//! For each facility, in the order of the job: one search from the facility
//! (same cost model as travel time) groups the reached cells into
//! travel-time bands; the residual population of each band decides the
//! catchment; the population covered is removed from the residual
//! population before the next facility.

use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::capacity::{Band, Catchment, catchment};
use accessmod_core::cost::{CellMobility, Direction, Method, Neighbourhood};
use accessmod_core::mobility::MobilityModel;
use accessmod_core::solver::{Facility, Flow, InputCell, SolverCell, SolverConfig, solve_until};
use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use gdal::vector::{FieldValue, Geometry, OGRFieldType, OGRwkbGeometryType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Cancelled};
use crate::error::{Error, Result};
use crate::facilities::{LoadedFacilities, load_facilities};
use crate::grid::GridSpec;
use crate::inputs::InputSummary;
use crate::job::{
    FacilityIssues, FacilitySource, JobFile, MethodName, ScenarioEntry, absolutize, check_common,
    check_files, scenario_table,
};
use crate::output::{
    self, BLOCK, CsvWriter, FLOAT_NODATA, GeoTiff, ScratchFile, Staging, csv_field,
};
use crate::raster::{AlignedBand, for_each_block, for_each_window, open_aligned_raster};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::travel_time::{Landscape, landscape_warnings, load_landscape, screen_facilities};
use crate::vector::{FeatureWriter, VectorLayer, compare_values, number_attribute, polygonize};

/// Capacity table file name.
pub const CAPACITY_FILE: &str = "capacity.csv";
/// Residual population file name.
pub const RESIDUAL_FILE: &str = "population_residual.tif";
/// Catchment polygons file name.
pub const CATCHMENTS_FILE: &str = "catchments.fgb";

fn sixty() -> f64 {
    60.0
}

fn yes() -> bool {
    true
}

/// Geographic coverage job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverageJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Digital elevation model; it defines the grid.
    pub dem: PathBuf,
    /// Land-cover raster with integer classes, on the DEM lattice.
    pub land_cover: PathBuf,
    /// Population raster (people per cell), on the DEM lattice.
    pub population: PathBuf,
    /// Facility point layer.
    pub facilities: FacilitySource,
    /// Field holding the capacity (people a facility can serve); not used
    /// with `ignore_capacity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_field: Option<String>,
    /// Field holding the facility name, copied to the table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_field: Option<String>,
    /// Order in which facilities take their population.
    pub order: CoverageOrder,
    /// Mobility of each land-cover class. Classes not listed are barriers.
    pub scenario: Vec<ScenarioEntry>,
    /// Travel-time method.
    pub method: MethodName,
    /// Direction of travel.
    pub direction: Direction,
    /// Neighbours reachable in one step: 8 or 16.
    pub neighbourhood: Neighbourhood,
    /// Inclusive travel-time limit, seconds.
    pub limit_s: f64,
    /// Width of the travel-time bands, seconds (`band = ceil(t / band_s)`);
    /// AccessMod 5 uses minutes.
    #[serde(default = "sixty")]
    pub band_s: f64,
    /// Named mobility model (anisotropic method).
    #[serde(default)]
    pub mobility_model: MobilityModel,
    /// Cover all the population each facility reaches (case E).
    #[serde(default)]
    pub ignore_capacity: bool,
    /// Remove the population covered before the next facility.
    #[serde(default = "yes")]
    pub remove_covered: bool,
    /// Write the catchment polygons (`catchments.fgb`).
    #[serde(default = "yes")]
    pub catchments: bool,
    /// What to do with facilities outside the grid or on a barrier.
    #[serde(default)]
    pub facility_issues: FacilityIssues,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

/// Processing order of the facilities. Ties keep the order of the facility
/// identifiers (numbers numerically).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum CoverageOrder {
    /// By the values of a numeric field (AccessMod 5 "table order").
    Field {
        /// Field name.
        field: String,
        /// Largest first.
        #[serde(default)]
        descending: bool,
    },
    /// By the population each facility reaches within `limit_s` before any
    /// removal (AccessMod 5 "travel time" order).
    Population {
        /// Limit of the pre-analysis, seconds.
        limit_s: f64,
        /// Largest first.
        #[serde(default)]
        descending: bool,
    },
}

impl JobFile for CoverageJob {
    fn resolve_paths(mut self, base: &Path) -> Self {
        for path in [
            &mut self.dem,
            &mut self.land_cover,
            &mut self.population,
            &mut self.facilities.path,
            &mut self.output_dir,
        ] {
            absolutize(path, base);
        }
        self
    }
}

impl CoverageJob {
    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues = check_common(self.schema_version, &self.scenario, Some(self.limit_s));
        if self.facilities.id_field.trim().is_empty() {
            issues.push("facilities.id_field is empty".into());
        }
        if !self.ignore_capacity && self.capacity_field.is_none() {
            issues.push("capacity_field is required unless ignore_capacity is set".into());
        }
        if !(self.band_s.is_finite() && self.band_s > 0.0) {
            issues.push(format!(
                "band_s must be finite and > 0, got {}",
                self.band_s
            ));
        }
        if let CoverageOrder::Population { limit_s, .. } = self.order
            && !(limit_s.is_finite() && limit_s > 0.0)
        {
            issues.push(format!(
                "order.limit_s must be finite and > 0, got {limit_s}"
            ));
        }
        issues
    }

    /// Solver method.
    pub fn method(&self) -> Method {
        match self.method {
            MethodName::Isotropic => Method::Isotropic,
            MethodName::Anisotropic => Method::Anisotropic(self.mobility_model),
        }
    }

    fn solver_config(&self, grid: &GridSpec, limit_s: f64) -> SolverConfig {
        SolverConfig {
            method: self.method(),
            direction: self.direction,
            neighbourhood: self.neighbourhood,
            cell: grid.cell,
            limit_s: Some(limit_s),
        }
    }
}

/// A facility of the analysis.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverageFacility {
    /// Identifier.
    pub id: serde_json::Value,
    /// Name, if `name_field` is set.
    pub name: Option<String>,
    /// Capacity, unless ignored.
    pub capacity: Option<f64>,
    /// Value of the order field, if ordered by a field.
    pub order_value: Option<f64>,
    /// Cell.
    #[serde(skip)]
    pub cell: Facility,
}

/// Inputs loaded and validated, ready to compute.
#[derive(Debug)]
pub struct PreparedCoverage {
    /// Job with resolved paths.
    pub job: CoverageJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Solver inputs.
    pub inputs: TiledGrid<InputCell>,
    /// Scenario slots.
    pub scenario: Vec<CellMobility>,
    /// Facilities read (used and skipped).
    pub loaded: LoadedFacilities,
    /// Facilities used, in layer order.
    pub facilities: Vec<CoverageFacility>,
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
    job: CoverageJob,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedCoverage> {
    let started = Instant::now();
    let mut issues = job.check();
    issues.extend(check_files(
        &job.output_dir,
        [
            ("dem".to_owned(), job.dem.as_path()),
            ("land_cover".to_owned(), job.land_cover.as_path()),
            ("population".to_owned(), job.population.as_path()),
            ("facilities.path".to_owned(), job.facilities.path.as_path()),
        ],
    ));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let (slots, scenario) = scenario_table(&job.scenario);
    // The residual population grid (f64) shares the tile budget.
    let geometry_probe = TileGeometry::new(1, 1, accessmod_core::store::DEFAULT_TILE)?;
    let residual_tile = TiledGrid::<f64>::tile_bytes(geometry_probe) as u64;
    let Landscape {
        grid,
        mut inputs,
        summary,
        plan,
    } = load_landscape(
        &job.dem,
        &job.land_cover,
        &slots,
        settings,
        residual_tile,
        cancelled,
    )?;
    open_aligned_raster(&job.population, &grid, "population")?;
    let loaded = load_facilities(&job.facilities, &grid, &mut inputs, cancelled)?;
    let mut warnings = Vec::new();
    screen_facilities(&loaded, job.facility_issues, "facilities", &mut warnings)?;
    warnings.extend(landscape_warnings(&summary, &job.scenario));
    let facilities = read_facilities(&job, &grid, &loaded, cancelled)?;
    let mut durations_s = BTreeMap::new();
    durations_s.insert("load_inputs", started.elapsed().as_secs_f64());
    let started = Instant::now();
    let fingerprints = fingerprints(
        &[
            ("dem", job.dem.as_path()),
            ("land_cover", job.land_cover.as_path()),
            ("population", job.population.as_path()),
            ("facilities", job.facilities.path.as_path()),
        ],
        cancelled,
    )?;
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());
    Ok(PreparedCoverage {
        job,
        grid,
        inputs,
        scenario,
        loaded,
        facilities,
        summary,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// Capacity, name and order value of the facilities used.
fn read_facilities(
    job: &CoverageJob,
    grid: &GridSpec,
    loaded: &LoadedFacilities,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<CoverageFacility>> {
    let source = &job.facilities;
    let vector = VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        "facilities",
    )?;
    let order_field = match &job.order {
        CoverageOrder::Field { field, .. } => Some(field.as_str()),
        CoverageOrder::Population { .. } => None,
    };
    let capacity_field = job
        .capacity_field
        .as_deref()
        .filter(|_| !job.ignore_capacity);
    let fields: Vec<&str> = [capacity_field, job.name_field.as_deref(), order_field]
        .into_iter()
        .flatten()
        .collect();
    let rows = vector.read_fields(&fields, cancelled)?;
    let mut issues = Vec::new();
    let mut facilities = Vec::with_capacity(loaded.kept.len());
    for record in &loaded.kept {
        let values = &rows[record.feature_index as usize];
        // Positions of the fields read: capacity, name, order (when set).
        let at_name = usize::from(capacity_field.is_some());
        let at_order = at_name + usize::from(job.name_field.is_some());
        // Capacities are >= 0; order values any finite number.
        let mut number = |field: Option<&str>, at: usize, what: &str, min: f64| -> Option<f64> {
            let field = field?;
            let value = values[at].clone().and_then(number_attribute);
            match value {
                Some(v) if v.is_finite() && v >= min => Some(v),
                _ => {
                    issues.push(format!(
                        "facility {}: {what} {field:?} must be a number >= {min}",
                        record.id
                    ));
                    None
                }
            }
        };
        let capacity = number(capacity_field, 0, "capacity field", 0.0);
        let order_value = number(order_field, at_order, "order field", f64::MIN);
        let name = match job.name_field.as_deref() {
            Some(_) => values[at_name].clone().map(|v| match v {
                FieldValue::StringValue(s) => s,
                other => crate::vector::json_attribute(Some(other))
                    .map(|j| j.to_string())
                    .unwrap_or_default(),
            }),
            None => None,
        };
        facilities.push(CoverageFacility {
            id: record.id.clone(),
            name,
            capacity,
            order_value,
            cell: Facility {
                row: record.row,
                col: record.col,
            },
        });
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    Ok(facilities)
}

/// Reached cells of one search, spooled to a scratch file so that memory
/// does not grow with the catchment: cell index and band.
struct CellSpool {
    file: ScratchFile,
    writer: Option<BufWriter<std::fs::File>>,
    count: u64,
}

impl CellSpool {
    fn new(dir: &Path) -> Result<Self> {
        let mut spool = Self {
            file: ScratchFile::new(dir, "coverage", "cells"),
            writer: None,
            count: 0,
        };
        spool.clear()?;
        Ok(spool)
    }

    fn path(&self) -> &Path {
        self.file.path()
    }

    fn clear(&mut self) -> Result<()> {
        let file = std::fs::File::create(self.path()).map_err(Error::file(self.path()))?;
        self.writer = Some(BufWriter::new(file));
        self.count = 0;
        Ok(())
    }

    fn push(&mut self, cell: u64, band: u32) -> std::io::Result<()> {
        let writer = self.writer.as_mut().expect("spool open for writing");
        writer.write_all(&cell.to_le_bytes())?;
        writer.write_all(&band.to_le_bytes())?;
        self.count += 1;
        Ok(())
    }

    /// Visit the spooled cells in the order they were pushed; cancellation
    /// is checked every 65 536 cells.
    fn for_each(
        &mut self,
        cancelled: &dyn Fn() -> bool,
        mut visit: impl FnMut(u64, u32) -> Result<()>,
    ) -> Result<()> {
        if let Some(mut writer) = self.writer.take() {
            writer.flush().map_err(Error::file(self.path()))?;
        }
        let file = std::fs::File::open(self.path()).map_err(Error::file(self.path()))?;
        let mut reader = BufReader::new(file);
        let mut entry = [0u8; 12];
        for k in 0..self.count {
            if k % 65_536 == 0 && cancelled() {
                return Err(Error::Cancelled);
            }
            reader
                .read_exact(&mut entry)
                .map_err(Error::file(self.path()))?;
            let cell = u64::from_le_bytes(entry[..8].try_into().expect("8 bytes"));
            let band = u32::from_le_bytes(entry[8..].try_into().expect("4 bytes"));
            visit(cell, band)?;
        }
        Ok(())
    }
}

/// Residual population per band of one search from `facility`, the
/// reached cells spooled. Cells without population are reached but hold no
/// residual: bands where no cell holds a value are left out (AccessMod 5).
#[allow(clippy::too_many_arguments)] // reason: one private helper, explicit inputs
fn search_bands(
    inputs: &mut TiledGrid<InputCell>,
    scenario: &[CellMobility],
    facility: Facility,
    config: &SolverConfig,
    band_s: f64,
    residual: &mut TiledGrid<f64>,
    spool: &mut CellSpool,
    state_tiles: usize,
    tmp_dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Band>> {
    let geometry = inputs.geometry();
    let cols = geometry.cols();
    let mut state = TiledGrid::new(
        geometry,
        SolverCell::UNREACHED,
        state_tiles.max(1),
        tmp_dir,
        "coverage",
    )?;
    spool.clear()?;
    let mut sums: BTreeMap<u32, f64> = BTreeMap::new();
    let mut failure: Option<Error> = None;
    solve_until(
        inputs,
        scenario,
        &[facility],
        config,
        &mut state,
        cancelled,
        &mut |r, c, cell| {
            let band = (cell.time_s / band_s).ceil() as u32;
            let result = spool
                .push((r * cols + c) as u64, band)
                .map_err(Error::file(spool.path()))
                .and_then(|()| residual.get(r, c).map_err(Error::from));
            match result {
                Ok(value) if !value.is_nan() => *sums.entry(band).or_default() += value,
                Ok(_) => {}
                Err(error) => {
                    failure = Some(error);
                    return Flow::Stop;
                }
            }
            Flow::Continue
        },
    )?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(sums
        .into_iter()
        .map(|(band, population)| Band { band, population })
        .collect())
}

/// Polygon of the spooled cells within band `limit`: their bounding box is
/// burnt into a disk-backed mask, written to a scratch GeoTIFF and
/// polygonised. Memory stays bounded by the tile cache and one polygon.
fn catchment_polygon(
    spool: &mut CellSpool,
    limit: u32,
    grid: &GridSpec,
    tmp_dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Geometry>> {
    let cols = grid.cols as u64;
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    spool.for_each(cancelled, |cell, band| {
        if band <= limit {
            let (r, c) = ((cell / cols) as usize, (cell % cols) as usize);
            bbox = Some(bbox.map_or((r, r, c, c), |(r0, r1, c0, c1)| {
                (r0.min(r), r1.max(r), c0.min(c), c1.max(c))
            }));
        }
        Ok(())
    })?;
    let Some((r0, r1, c0, c1)) = bbox else {
        return Ok(None);
    };
    let window = grid.window(r0..r1 + 1, c0..c1 + 1);
    let geometry = TileGeometry::new(
        window.rows,
        window.cols,
        accessmod_core::store::DEFAULT_TILE,
    )?;
    // A small cache: the mask is written once in spool order, then read
    // block by block.
    let mut mask = TiledGrid::new(geometry, 0u8, 64, tmp_dir, "catchment")?;
    spool.for_each(cancelled, |cell, band| {
        if band <= limit {
            let (r, c) = ((cell / cols) as usize, (cell % cols) as usize);
            mask.set(r - r0, c - c0, 1)?;
        }
        Ok(())
    })?;
    let scratch = ScratchFile::new(tmp_dir, "catchment", "tif");
    let path = scratch.path().to_path_buf();
    let tif = GeoTiff::create::<u8>(&path, &window, 2, 1, 0.0)?;
    let mut cells = vec![0u8; BLOCK];
    let mut block = Vec::with_capacity(BLOCK * BLOCK);
    for_each_window(
        0..window.rows,
        0..window.cols,
        (BLOCK, BLOCK),
        cancelled,
        |r, c| {
            for row in r.clone() {
                mask.read_row(row, c.start, &mut cells[..c.len()])?;
                block.extend_from_slice(&cells[..c.len()]);
            }
            tif.write(r, c, &mut block)
        },
    )?;
    tif.close()?;
    let dataset = gdal::Dataset::open(&path).map_err(Error::gdal(&path))?;
    let polygon = polygonize(&dataset, &path);
    drop(dataset);
    polygon
}

/// One row of the capacity table.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverageRow {
    /// Processing order, from 1.
    pub order: usize,
    /// Facility.
    pub facility: CoverageFacility,
    /// Catchment.
    #[serde(skip)]
    pub catchment: Catchment,
    /// Population covered so far, percent of the population off barriers.
    pub population_covered_percent: f64,
}

/// Result counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CoverageSummary {
    /// Facilities processed.
    pub facilities: usize,
    /// Population off barriers at start.
    pub population: f64,
    /// Residual population at the end.
    pub population_residual: f64,
    /// Population covered, percent.
    pub population_covered_percent: f64,
    /// Cells reached over all searches.
    pub reached_cells: u64,
}

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageRecord {
    /// Job with resolved paths and defaults.
    pub job: CoverageJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// DEM grid.
    pub grid: GridReport,
    /// Input counts.
    pub input_summary: InputSummary,
    /// Totals.
    pub result: CoverageSummary,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Execution details.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageStores {
    /// Cache counters of the input grid.
    pub input_store: StoreStats,
    /// Cache counters of the residual population grid.
    pub residual_store: StoreStats,
}

/// `run.json` of a geographic coverage.
pub type CoverageReport = Report<CoverageRecord, CoverageStores>;

/// Deviations from AccessMod 5 that apply to every run.
pub fn deviations() -> Vec<Deviation> {
    vec![
        Deviation {
            code: "dem_nodata_barrier",
            message: "DEM NoData cells are barriers (see travel time).",
        },
        Deviation {
            code: "coverage_order_ties",
            message: "Facilities with equal order values are taken in the order of their \
                      identifiers. AccessMod 5 keeps the order of its facility table.",
        },
        Deviation {
            code: "coverage_not_yet",
            message: "Not yet available: circular-buffer order, \
                      population on barriers map, zonal coverage table.",
        },
    ]
}

/// Compute the coverage and write every output atomically.
pub fn run(
    prepared: PreparedCoverage,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<(PathBuf, CoverageReport)> {
    let PreparedCoverage {
        job,
        grid,
        mut inputs,
        scenario,
        facilities,
        summary: input_summary,
        warnings,
        plan,
        fingerprints,
        mut durations_s,
        ..
    } = prepared;
    let staging = Staging::create(&job.output_dir)?;
    let geometry = inputs.geometry();
    let mut residual = TiledGrid::new(
        geometry,
        f64::NAN,
        plan.resident_tiles,
        &settings.tmp_dir,
        "residual",
    )?;

    // Residual population (AccessMod 5 amInitPopResidual, `pop > 0 &&& speed
    // > 0 ? pop : 0` with the null-aware `&&&`): no value without
    // population, 0 for people <= 0 (barriers included), the people off
    // barriers, no value for people on barriers.
    let started = Instant::now();
    let (population_ds, alignment) = open_aligned_raster(&job.population, &grid, "population")?;
    let population = AlignedBand::new(&population_ds, &job.population, alignment, &grid)?;
    let mut total = 0.0;
    for_each_block(
        &grid,
        (BLOCK, BLOCK),
        &[&population],
        None,
        cancelled,
        |block| {
            let width = block.cols.len();
            for i in 0..block.len() {
                let (row, col) = (block.rows.start + i / width, block.cols.start + i % width);
                let Some(people) = block.value(0, i) else {
                    continue;
                };
                let value = if people <= 0.0 {
                    0.0
                } else if inputs.get(row, col)?.is_passable() {
                    people
                } else {
                    continue;
                };
                total += value;
                residual.set(row, col, value)?;
            }
            Ok(())
        },
    )?;
    durations_s.insert("residual", started.elapsed().as_secs_f64());

    let mut spool = CellSpool::new(&settings.tmp_dir)?;
    let state_tiles = plan.resident_tiles;

    // Processing order.
    let started = Instant::now();
    let mut order: Vec<usize> = (0..facilities.len()).collect();
    let (values, descending): (Vec<f64>, bool) = match &job.order {
        CoverageOrder::Field { descending, .. } => (
            facilities
                .iter()
                .map(|f| f.order_value.unwrap_or(0.0))
                .collect(),
            *descending,
        ),
        CoverageOrder::Population {
            limit_s,
            descending,
        } => {
            let config = job.solver_config(&grid, *limit_s);
            let mut values = Vec::with_capacity(facilities.len());
            for facility in &facilities {
                let bands = search_bands(
                    &mut inputs,
                    &scenario,
                    facility.cell,
                    &config,
                    job.band_s,
                    &mut residual,
                    &mut spool,
                    state_tiles,
                    &settings.tmp_dir,
                    cancelled,
                )?;
                values.push(bands.iter().map(|b| b.population).sum());
            }
            (values, *descending)
        }
    };
    order.sort_by(|&a, &b| {
        let by_value = values[a].total_cmp(&values[b]);
        let by_value = if descending {
            by_value.reverse()
        } else {
            by_value
        };
        by_value.then_with(|| compare_values(&facilities[a].id, &facilities[b].id))
    });
    durations_s.insert("order", started.elapsed().as_secs_f64());

    // Facilities one after the other.
    let started = Instant::now();
    let config = job.solver_config(&grid, job.limit_s);
    let limit_band = (job.limit_s / job.band_s).ceil() as u32;
    let mut catchments = if job.catchments {
        Some(FeatureWriter::create(
            &staging.path().join(CATCHMENTS_FILE),
            "catchments",
            &grid.srs,
            OGRwkbGeometryType::wkbMultiPolygon,
            &[
                ("order", OGRFieldType::OFTInteger),
                ("facility_id", OGRFieldType::OFTString),
                ("name", OGRFieldType::OFTString),
                ("case", OGRFieldType::OFTString),
            ],
        )?)
    } else {
        None
    };
    let mut remaining = total;
    let mut reached_cells = 0u64;
    let mut rows = Vec::with_capacity(order.len());
    for (k, &index) in order.iter().enumerate() {
        let facility = &facilities[index];
        let bands = search_bands(
            &mut inputs,
            &scenario,
            facility.cell,
            &config,
            job.band_s,
            &mut residual,
            &mut spool,
            state_tiles,
            &settings.tmp_dir,
            cancelled,
        )?;
        reached_cells += spool.count;
        let capacity = if job.ignore_capacity {
            None
        } else {
            facility.capacity
        };
        let result = catchment(&bands, capacity, limit_band);
        if let (Some(writer), Some(limit)) = (catchments.as_mut(), result.band_limit)
            && let Some(polygon) =
                catchment_polygon(&mut spool, limit, &grid, &settings.tmp_dir, cancelled)?
        {
            writer.write(
                polygon,
                &[
                    Some(FieldValue::IntegerValue(k as i32 + 1)),
                    Some(FieldValue::StringValue(csv_field(&facility.id))),
                    facility.name.clone().map(FieldValue::StringValue),
                    result
                        .case
                        .map(|c| FieldValue::StringValue(format!("{c:?}"))),
                ],
            )?;
        }
        if job.remove_covered {
            let (inner, outer) = (result.inner, result.outer);
            spool.for_each(cancelled, |cell, band| {
                let (row, col) = (
                    (cell / geometry.cols() as u64) as usize,
                    (cell % geometry.cols() as u64) as usize,
                );
                let value = residual.get(row, col)?;
                let new = if inner.is_some_and(|i| band <= i) {
                    // AccessMod 5 sets 0 even where there is no population.
                    0.0
                } else if let Some((_, fraction)) = outer.filter(|(b, _)| *b == band) {
                    value - value * fraction
                } else {
                    return Ok(());
                };
                if !value.is_nan() {
                    remaining -= value - new;
                }
                residual.set(row, col, new)?;
                Ok(())
            })?;
        }
        let percent = if total > 0.0 {
            (total - remaining) / total * 100.0
        } else {
            0.0
        };
        rows.push(CoverageRow {
            order: k + 1,
            facility: facility.clone(),
            catchment: result,
            population_covered_percent: percent,
        });
    }
    if let Some(writer) = catchments {
        writer.close()?;
    }
    durations_s.insert("coverage", started.elapsed().as_secs_f64());

    // Outputs.
    let started = Instant::now();
    let tif = GeoTiff::create::<f64>(
        &staging.path().join(RESIDUAL_FILE),
        &grid,
        3,
        settings.threads,
        FLOAT_NODATA,
    )?;
    let mut cells = vec![0.0f64; BLOCK];
    let mut block = Vec::with_capacity(BLOCK * BLOCK);
    for_each_window(
        0..grid.rows,
        0..grid.cols,
        (BLOCK, BLOCK),
        cancelled,
        |r, c| {
            for row in r.clone() {
                residual.read_row(row, c.start, &mut cells[..c.len()])?;
                block.extend(
                    cells[..c.len()]
                        .iter()
                        .map(|v| if v.is_nan() { FLOAT_NODATA } else { *v }),
                );
            }
            tif.write(r, c, &mut block)
        },
    )?;
    tif.close()?;
    write_capacity_table(&staging.path().join(CAPACITY_FILE), &job, &rows)?;
    durations_s.insert("write_outputs", started.elapsed().as_secs_f64());

    let summary = CoverageSummary {
        facilities: rows.len(),
        population: total,
        population_residual: remaining,
        population_covered_percent: rows.last().map_or(0.0, |r| r.population_covered_percent),
        reached_cells,
    };
    let stores = CoverageStores {
        input_store: inputs.stats(),
        residual_store: residual.stats(),
    };
    drop(spool);
    let report = Report::new(
        CoverageRecord {
            job,
            inputs: fingerprints,
            grid: GridReport::of(&grid),
            input_summary,
            result: summary,
            warnings,
            deviations: deviations(),
        },
        Execution::now(settings, plan, durations_s, stores),
    );
    let output_dir = output::finish(staging, &report.run.job, &report, cancelled)?;
    Ok((output_dir, report))
}

fn write_capacity_table(path: &Path, job: &CoverageJob, rows: &[CoverageRow]) -> Result<()> {
    let mut csv = CsvWriter::create(
        path,
        "order,facility_id,name,capacity,order_value,travel_time_limit_s,\
         population_within_limit,population_band_correlation,catchment_travel_time_s,\
         population_catchment,capacity_realised,capacity_residual,\
         population_catchment_not_covered,population_covered_percent,catchment_case",
    )?;
    let number = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_default();
    for row in rows {
        let c = &row.catchment;
        let f = &row.facility;
        csv.record(&[
            row.order.to_string(),
            csv_field(&f.id),
            f.name
                .as_ref()
                .map(|n| csv_field(&serde_json::Value::String(n.clone())))
                .unwrap_or_default(),
            number(Some(c.capacity)),
            number(f.order_value),
            number(Some(job.limit_s)),
            number(c.population_within_limit),
            number(c.correlation),
            number(c.band_limit.map(|b| f64::from(b) * job.band_s)),
            number(c.population_catchment),
            number(Some(c.capacity_realised)),
            number(Some(c.capacity_residual)),
            number(c.population_not_covered()),
            number(Some(row.population_covered_percent)),
            c.case.map(|k| format!("{k:?}")).unwrap_or_default(),
        ])?;
    }
    csv.finish()
}

/// Geographic coverage.
#[derive(Debug)]
pub struct GeographicCoverage;

impl Analysis for GeographicCoverage {
    type Job = CoverageJob;
    type Prepared = PreparedCoverage;
    type Report = CoverageReport;

    fn prepare(
        job: CoverageJob,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<PreparedCoverage> {
        prepare(job, settings, cancelled)
    }

    fn run(
        prepared: PreparedCoverage,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<(PathBuf, CoverageReport)> {
        run(prepared, settings, cancelled)
    }

    fn warnings(prepared: &PreparedCoverage) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &PreparedCoverage) -> String {
        format!(
            "geographic coverage: {} facilities on a {} x {} grid",
            prepared.facilities.len(),
            prepared.grid.cols,
            prepared.grid.rows
        )
    }

    fn validation(prepared: &PreparedCoverage) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "facilities": prepared.facilities,
            "facilities_skipped": prepared.loaded.issues,
            "input_summary": prepared.summary,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &CoverageReport) -> String {
        let r = &report.run.result;
        format!(
            "{} facilities cover {:.2} % of {} people",
            r.facilities, r.population_covered_percent, r.population
        )
    }
}
