//! Scaling up (schema version 1): add facilities one at a time where they
//! are most suitable and cover the most people, as AccessMod 5 does
//! (`tools/R/amAnalysisScalingUp.R`, rules in `accessmod_core::scaling_up`).
//!
//! Each iteration:
//!
//! 1. exclusion rules remove candidate cells (around facilities, inside or
//!    outside layers); candidates start as every cell that can be entered;
//! 2. suitability factors (population, distance, travel time, priority)
//!    are rescaled over the candidates and combined; the cells of highest
//!    suitability are evaluated one by one (row-major order);
//! 3. the cell reaching the most residual population within the limit
//!    becomes a facility of the capacity class matching that population;
//!    its catchment is removed from the residual population, as in the
//!    geographic coverage.
//!
//! It stops after `max_facilities` iterations, once the population covered
//! reaches `max_coverage_percent`, after `max_duration_s`, or when no
//! candidate is left.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::capacity::catchment;
use accessmod_core::cost::{CellMobility, Direction, Method, Neighbourhood};
use accessmod_core::focal::{circular_sum, distance_to_sources};
use accessmod_core::mobility::MobilityModel;
use accessmod_core::scaling_up::{
    CapacityClass, Rescale, capacity_class, round_half_even, suitability,
};
use accessmod_core::solver::{Facility, Flow, InputCell, SolverCell, SolverConfig, solve_until};
use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use gdal::vector::{FieldValue, Geometry, OGRFieldType, OGRwkbGeometryType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, LintContext, Monitor, Output};
use crate::catchment::{
    CATCHMENT_COLUMNS, CellSpool, PopulationTotals, ResidualStep, catchment_fields,
    catchment_polygon, init_residual, remove_covered, search_bands, table_number,
};
use crate::class::DataClass;
use crate::error::{Error, Result};
use crate::facilities::{LoadedFacilities, load_facilities};
use crate::grid::GridSpec;
use crate::inputs::InputSummary;
use crate::job::{
    FacilityIssues, FacilitySource, JobFile, MethodName, ScenarioEntry, check_common, check_files,
    scenario_table,
};
use crate::output::{self, BLOCK, CsvWriter, FLOAT_NODATA, GeoTiff, Staging, csv_field};
use crate::raster::{AlignedBand, for_each_block, for_each_window, open_aligned_raster};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::travel_time::{
    Landscape, landscape_warnings, lint_landscape, load_landscape, screen_facilities,
};
use crate::vector::{Burn, FeatureWriter, VectorLayer, burn_layer};

/// Capacity table file name.
pub const CAPACITY_FILE: &str = "capacity.csv";
/// New facilities file name.
pub const FACILITIES_FILE: &str = "facilities.fgb";
/// Residual population file name.
pub const RESIDUAL_FILE: &str = "population_residual.tif";
/// Catchment polygons file name.
pub const CATCHMENTS_FILE: &str = "catchments.fgb";

/// Travel times beyond this many minutes have no value (AccessMod 5
/// unlimited travel times, `int16Max`).
const MAX_MINUTES: f64 = 32_767.0;

fn sixty() -> f64 {
    60.0
}

fn hundred() -> f64 {
    100.0
}

fn yes() -> bool {
    true
}

fn towards() -> Direction {
    Direction::TowardsFacilities
}

fn eight() -> Neighbourhood {
    Neighbourhood::Eight
}

/// Scaling-up job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScalingUpJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Digital elevation model; it defines the grid.
    #[schemars(extend("x-accessmod-class" = ["rDem"]))]
    pub dem: PathBuf,
    /// Land-cover raster with integer classes, on the DEM lattice.
    #[schemars(extend("x-accessmod-class" = ["rLandCoverMerged"]))]
    pub land_cover: PathBuf,
    /// Population raster (people per cell): the population the covered
    /// percent refers to.
    #[schemars(extend("x-accessmod-class" = ["rPopulation", "rPopulationResidual"]))]
    pub population: PathBuf,
    /// Population to cover at start (e.g. the residual of a coverage);
    /// `population` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-accessmod-class" = ["rPopulationResidual", "rPopulation"]))]
    pub population_residual: Option<PathBuf>,
    /// Existing facilities, kept: they count in the `facilities` layer of
    /// the rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-accessmod-class" = ["vFacility", "vFacilityNew"]))]
    pub facilities: Option<FacilitySource>,
    /// Mobility of each land-cover class. Classes not listed are barriers.
    pub scenario: Vec<ScenarioEntry>,
    /// Travel-time method of the candidate evaluations and catchments.
    pub method: MethodName,
    /// Direction of travel.
    #[serde(default = "towards")]
    pub direction: Direction,
    /// Neighbours reachable in one step: 8 or 16.
    pub neighbourhood: Neighbourhood,
    /// Inclusive travel-time limit of the catchments, seconds.
    pub limit_s: f64,
    /// Width of the travel-time bands, seconds; AccessMod 5 uses minutes.
    #[serde(default = "sixty")]
    pub band_s: f64,
    /// Named mobility model (anisotropic method).
    #[serde(default)]
    pub mobility_model: MobilityModel,
    /// Facility classes: the population a new facility reaches selects its
    /// capacity.
    pub capacities: Vec<CapacityEntry>,
    /// Exclusion rules, applied in order at each iteration.
    #[serde(default)]
    pub exclusions: Vec<Exclusion>,
    /// Suitability factors, combined in order.
    pub suitability: Vec<Factor>,
    /// Facilities to add at most (iterations).
    pub max_facilities: u32,
    /// Stop once this percent of the population is covered.
    #[serde(default = "hundred")]
    pub max_coverage_percent: f64,
    /// Stop after this many seconds (results then depend on the machine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_s: Option<f64>,
    /// Write the catchment polygons (`catchments.fgb`).
    #[serde(default = "yes")]
    pub catchments: bool,
    /// What to do with existing facilities outside the grid or on a barrier.
    #[serde(default)]
    pub facility_issues: FacilityIssues,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

/// A facility class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapacityEntry {
    /// Smallest population reached for this class.
    pub min: f64,
    /// Largest population reached for this class.
    pub max: f64,
    /// Capacity of a new facility of this class.
    pub capacity: f64,
    /// Class name.
    pub label: String,
}

/// A layer used by a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum Layer {
    /// The facilities: existing ones and those added so far.
    Facilities,
    /// The residual population (population factor only).
    Population,
    /// A raster on the DEM lattice: its cells with a value.
    Raster(PathBuf),
    /// A vector layer: its points, lines (thin) and polygons.
    Vector(VectorSource),
}

impl Layer {
    fn path_mut(&mut self) -> Option<&mut PathBuf> {
        match self {
            Layer::Raster(path) => Some(path),
            Layer::Vector(source) => Some(&mut source.path),
            _ => None,
        }
    }

    fn path(&self) -> Option<&Path> {
        match self {
            Layer::Raster(path) => Some(path),
            Layer::Vector(source) => Some(&source.path),
            _ => None,
        }
    }
}

/// A vector layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VectorSource {
    /// Vector dataset.
    pub path: PathBuf,
    /// Layer name; the first layer when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Optional OGR SQL attribute filter.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// Cells kept by an exclusion rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
    /// Within `buffer_m` of the layer.
    Inside,
    /// Farther than `buffer_m` from the layer.
    Outside,
}

/// An exclusion rule (AccessMod 5 exclusion table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    /// Layer: `facilities`, a raster or a vector layer.
    #[schemars(extend("x-accessmod-class" = ["rExclusion", "vExclusion"], "x-accessmod-dynamic" = ["facilities"]))]
    pub layer: Layer,
    /// Distance from the layer's cells, metres (cell centre to cell
    /// centre, inclusive); 0: the layer's cells.
    #[serde(default)]
    pub buffer_m: f64,
    /// Candidates kept.
    pub keep: Keep,
}

/// Which values a factor favours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Prefer {
    /// High values are most suitable.
    High,
    /// Low values are most suitable.
    Low,
}

/// A suitability factor (AccessMod 5 suitability table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "factor", rename_all = "snake_case", deny_unknown_fields)]
pub enum Factor {
    /// Population summed over a moving window (`popsum`).
    Population {
        /// `population` (residual) or a raster.
        #[schemars(extend("x-accessmod-class" = ["rPopulation", "rPopulationResidual"], "x-accessmod-dynamic" = ["population"]))]
        layer: Layer,
        /// Width of the window, metres: `round(window_m / cell)` cells,
        /// made odd, circular (AccessMod 5 option `r`, in km); 0: the cell.
        #[serde(default)]
        window_m: f64,
        /// Weight.
        weight: f64,
        /// Favoured values.
        prefer: Prefer,
    },
    /// Euclidean distance to the layer (`dist`).
    Distance {
        /// `facilities`, a raster or a vector layer.
        #[schemars(extend("x-accessmod-class" = ["vRoad", "vBarrier", "vFacility"], "x-accessmod-dynamic" = ["facilities"]))]
        layer: Layer,
        /// Weight.
        weight: f64,
        /// Favoured values.
        prefer: Prefer,
    },
    /// Travel time from the layer, whole minutes, unlimited (`traveltime`).
    TravelTime {
        /// `facilities` or a vector layer.
        #[schemars(extend("x-accessmod-class" = ["vRoad", "vBarrier", "vFacility"], "x-accessmod-dynamic" = ["facilities"]))]
        layer: Layer,
        /// Travel-time method.
        method: MethodName,
        /// Direction of travel.
        #[serde(default = "towards")]
        direction: Direction,
        /// Neighbours reachable in one step.
        #[serde(default = "eight")]
        neighbourhood: Neighbourhood,
        /// Weight.
        weight: f64,
        /// Favoured values.
        prefer: Prefer,
    },
    /// Values of a raster (`priority`).
    Priority {
        /// A raster.
        #[schemars(extend("x-accessmod-class" = ["rPriority"], "x-accessmod-dynamic" = []))]
        layer: Layer,
        /// Weight.
        weight: f64,
        /// Favoured values.
        prefer: Prefer,
    },
}

impl Factor {
    /// Layer of the factor.
    pub fn layer(&self) -> &Layer {
        match self {
            Factor::Population { layer, .. }
            | Factor::Distance { layer, .. }
            | Factor::TravelTime { layer, .. }
            | Factor::Priority { layer, .. } => layer,
        }
    }

    fn layer_mut(&mut self) -> &mut Layer {
        match self {
            Factor::Population { layer, .. }
            | Factor::Distance { layer, .. }
            | Factor::TravelTime { layer, .. }
            | Factor::Priority { layer, .. } => layer,
        }
    }

    /// Weight of the factor.
    pub fn weight(&self) -> f64 {
        match self {
            Factor::Population { weight, .. }
            | Factor::Distance { weight, .. }
            | Factor::TravelTime { weight, .. }
            | Factor::Priority { weight, .. } => *weight,
        }
    }

    fn prefer(&self) -> Prefer {
        match self {
            Factor::Population { prefer, .. }
            | Factor::Distance { prefer, .. }
            | Factor::TravelTime { prefer, .. }
            | Factor::Priority { prefer, .. } => *prefer,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Factor::Population { .. } => "population",
            Factor::Distance { .. } => "distance",
            Factor::TravelTime { .. } => "travel_time",
            Factor::Priority { .. } => "priority",
        }
    }

    /// Layers allowed for this factor.
    fn allows(&self, layer: &Layer) -> bool {
        matches!(
            (self, layer),
            (
                Factor::Population { .. },
                Layer::Population | Layer::Raster(_)
            ) | (
                Factor::Distance { .. },
                Layer::Facilities | Layer::Raster(_) | Layer::Vector(_)
            ) | (
                Factor::TravelTime { .. },
                Layer::Facilities | Layer::Vector(_)
            ) | (Factor::Priority { .. }, Layer::Raster(_))
        )
    }
}

impl JobFile for ScalingUpJob {
    fn for_each_path(&mut self, f: &mut dyn FnMut(&mut PathBuf)) {
        for path in [
            &mut self.dem,
            &mut self.land_cover,
            &mut self.population,
            &mut self.output_dir,
        ] {
            f(path);
        }
        if let Some(path) = &mut self.population_residual {
            f(path);
        }
        if let Some(source) = &mut self.facilities {
            f(&mut source.path);
        }
        for exclusion in &mut self.exclusions {
            if let Some(path) = exclusion.layer.path_mut() {
                f(path);
            }
        }
        for factor in &mut self.suitability {
            if let Some(path) = factor.layer_mut().path_mut() {
                f(path);
            }
        }
    }
}

impl ScalingUpJob {
    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues = check_common(self.schema_version, &self.scenario, Some(self.limit_s));
        let positive = |x: f64| x.is_finite() && x > 0.0;
        if let Some(source) = &self.facilities
            && source.id_field.trim().is_empty()
        {
            issues.push("facilities.id_field is empty".into());
        }
        if !positive(self.band_s) {
            issues.push(format!(
                "band_s must be finite and > 0, got {}",
                self.band_s
            ));
        }
        if self.capacities.is_empty() {
            issues.push("capacities is empty".into());
        }
        for (k, c) in self.capacities.iter().enumerate() {
            if !(c.min.is_finite() && c.max.is_finite() && c.min <= c.max) {
                issues.push(format!(
                    "capacities[{k}]: min and max must be finite, min <= max"
                ));
            }
            if !(c.capacity.is_finite() && c.capacity >= 0.0) {
                issues.push(format!("capacities[{k}]: capacity must be finite and >= 0"));
            }
        }
        for (k, e) in self.exclusions.iter().enumerate() {
            if matches!(e.layer, Layer::Population) {
                issues.push(format!(
                    "exclusions[{k}]: the population layer cannot exclude"
                ));
            }
            if !(e.buffer_m.is_finite() && e.buffer_m >= 0.0) {
                issues.push(format!("exclusions[{k}]: buffer_m must be finite and >= 0"));
            }
        }
        if self.suitability.is_empty() {
            issues.push("suitability is empty".into());
        }
        if !self.suitability.iter().any(|f| f.weight() > 0.0) {
            issues.push("suitability: at least one factor needs a weight > 0".into());
        }
        for (k, f) in self.suitability.iter().enumerate() {
            if !(f.weight().is_finite() && f.weight() >= 0.0) {
                issues.push(format!("suitability[{k}]: weight must be finite and >= 0"));
            }
            if !f.allows(f.layer()) {
                issues.push(format!(
                    "suitability[{k}]: layer not allowed for the {} factor",
                    f.name()
                ));
            }
            if let Factor::Population { window_m, .. } = f
                && !(window_m.is_finite() && *window_m >= 0.0)
            {
                issues.push(format!(
                    "suitability[{k}]: window_m must be finite and >= 0"
                ));
            }
        }
        if self.max_facilities == 0 {
            issues.push("max_facilities must be >= 1".into());
        }
        if !(self.max_coverage_percent > 0.0 && self.max_coverage_percent <= 100.0) {
            issues.push("max_coverage_percent must be in (0, 100]".into());
        }
        if let Some(d) = self.max_duration_s
            && !positive(d)
        {
            issues.push("max_duration_s must be finite and > 0".into());
        }
        issues
    }

    fn solver_config(&self, grid: &GridSpec) -> SolverConfig {
        SolverConfig {
            method: method(self.method, self.mobility_model),
            direction: self.direction,
            neighbourhood: self.neighbourhood,
            cell: grid.cell,
            limit_s: Some(self.limit_s),
        }
    }

    /// Facilities feed a rule (distance, exclusion, travel time).
    fn uses_facilities(&self) -> bool {
        self.exclusions.iter().any(|e| e.layer == Layer::Facilities)
            || self
                .suitability
                .iter()
                .any(|f| f.weight() > 0.0 && *f.layer() == Layer::Facilities)
    }

    /// Factors with a weight > 0: AccessMod 5 drops the others before
    /// reading their layers.
    fn active_factors(&self) -> impl Iterator<Item = &Factor> {
        self.suitability.iter().filter(|f| f.weight() > 0.0)
    }

    /// Input files: role and path.
    fn files(&self) -> Vec<(&'static str, &Path)> {
        let mut files = vec![
            ("dem", self.dem.as_path()),
            ("land_cover", self.land_cover.as_path()),
            ("population", self.population.as_path()),
        ];
        if let Some(path) = &self.population_residual {
            files.push(("population_residual", path.as_path()));
        }
        if let Some(source) = &self.facilities {
            files.push(("facilities", source.path.as_path()));
        }
        for e in &self.exclusions {
            if let Some(path) = e.layer.path() {
                files.push(("exclusion_layer", path));
            }
        }
        for f in self.active_factors() {
            if let Some(path) = f.layer().path() {
                files.push(("suitability_layer", path));
            }
        }
        files
    }
}

fn method(name: MethodName, model: MobilityModel) -> Method {
    match name {
        MethodName::Isotropic => Method::Isotropic,
        MethodName::Anisotropic => Method::Anisotropic(model),
    }
}

/// Inputs loaded and validated, ready to compute.
#[derive(Debug)]
pub struct PreparedScalingUp {
    /// Job with resolved paths.
    pub job: ScalingUpJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Solver inputs.
    pub inputs: TiledGrid<InputCell>,
    /// Scenario slots.
    pub scenario: Vec<CellMobility>,
    /// Existing facilities read (used and skipped).
    pub loaded: Option<LoadedFacilities>,
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

/// Grids of one tile each the analysis keeps beside the solver: residual
/// population, candidates, facility distances, one per factor and a
/// scratch grid for static layers.
fn extra_per_tile(job: &ScalingUpJob) -> Result<u64> {
    let probe = TileGeometry::new(1, 1, accessmod_core::store::DEFAULT_TILE)?;
    let f64_tile = TiledGrid::<f64>::tile_bytes(probe) as u64;
    let u8_tile = TiledGrid::<u8>::tile_bytes(probe) as u64;
    let factor_grids = job
        .active_factors()
        .filter(|f| needs_factor_grid(f))
        .count() as u64;
    let distance = u64::from(job.uses_facilities());
    Ok(f64_tile * (2 + distance + factor_grids) + u8_tile)
}

/// Row buffers: the rows a population window holds, and a few rows of the
/// grid width for the row passes (distance transform, exclusions, copies).
fn extra_bytes(job: &ScalingUpJob, grid: &GridSpec) -> u64 {
    let row = (grid.cols * 8) as u64;
    let window_rows: u64 = job
        .active_factors()
        .map(|f| match f {
            Factor::Population { window_m, .. } => {
                window_cells(*window_m, grid.cell.ns_m()).min(grid.rows) as u64 + 1
            }
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    // Distance transform: values, nearest rows (16 bytes), sites, bounds,
    // output; exclusions and copies use two more.
    row * (window_rows + 8)
}

/// A factor whose values are stored in their own grid (the others read the
/// residual population or the facility distances).
fn needs_factor_grid(factor: &Factor) -> bool {
    match factor {
        Factor::Population {
            layer, window_m, ..
        } => *layer != Layer::Population || *window_m > 0.0,
        Factor::Distance { layer, .. } => *layer != Layer::Facilities,
        _ => true,
    }
}

/// Moving-window size, cells (AccessMod 5: `round(r / resolution)`, made
/// odd; 0: no window).
fn window_cells(window_m: f64, resolution_m: f64) -> usize {
    let size = round_half_even(window_m / resolution_m) as usize;
    if size > 0 && size.is_multiple_of(2) {
        size + 1
    } else {
        size
    }
}

/// Validate the job against its inputs and load them.
pub fn prepare(
    job: ScalingUpJob,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<PreparedScalingUp> {
    let cancelled = monitor.cancelled();
    monitor.stage("load_inputs");
    let started = Instant::now();
    let mut issues = job.check();
    issues.extend(check_files(
        &job.output_dir,
        job.files()
            .into_iter()
            .map(|(role, path)| (role.to_owned(), path)),
    ));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let (slots, scenario) = scenario_table(&job.scenario);
    let (_, dem_grid) = GridSpec::open_dem(&job.dem)?;
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
        extra_per_tile(&job)?,
        extra_bytes(&job, &dem_grid),
        cancelled,
    )?;
    let mut issues = Vec::new();
    let rasters = std::iter::once(("population", job.population.as_path()))
        .chain(
            job.population_residual
                .as_deref()
                .map(|p| ("population_residual", p)),
        )
        .chain(
            job.exclusions
                .iter()
                .map(|e| &e.layer)
                .chain(job.active_factors().map(|f| f.layer()))
                .filter_map(|l| match l {
                    Layer::Raster(path) => Some(("layer", path.as_path())),
                    _ => None,
                }),
        );
    for (role, path) in rasters {
        match open_aligned_raster(path, &grid, role) {
            Ok(_) => {}
            Err(Error::Validation { issues: more }) => issues.extend(more),
            Err(other) => return Err(other),
        }
    }
    for layer in job
        .exclusions
        .iter()
        .map(|e| &e.layer)
        .chain(job.active_factors().map(|f| f.layer()))
    {
        if let Layer::Vector(source) = layer {
            match open_vector(source, &grid) {
                Ok(_) => {}
                Err(Error::Validation { issues: more }) => issues.extend(more),
                Err(other) => return Err(other),
            }
        }
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let mut warnings = Vec::new();
    let loaded = match &job.facilities {
        Some(source) => {
            let loaded = load_facilities(source, &grid, &mut inputs, cancelled)?;
            screen_facilities(&loaded, job.facility_issues, "facilities", &mut warnings)?;
            Some(loaded)
        }
        None => None,
    };
    warnings.extend(landscape_warnings(&summary, &job.scenario));
    if job.max_duration_s.is_some() {
        warnings.push(Warning {
            code: "duration_limit",
            message: "max_duration_s is set: the facilities added depend on the machine speed"
                .into(),
        });
    }
    let mut durations_s = BTreeMap::new();
    durations_s.insert("load_inputs", started.elapsed().as_secs_f64());
    monitor.stage("fingerprints");
    let started = Instant::now();
    let fingerprints = fingerprints(&job.files(), cancelled)?;
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());
    Ok(PreparedScalingUp {
        job,
        grid,
        inputs,
        scenario,
        loaded,
        summary,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

fn open_vector(source: &VectorSource, grid: &GridSpec) -> Result<VectorLayer> {
    VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        "layer",
    )
}

/// Why the iterations stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// `max_facilities` iterations done.
    MaxFacilities,
    /// `max_coverage_percent` reached.
    Coverage,
    /// `max_duration_s` reached.
    Duration,
    /// No candidate cell left after the exclusions.
    NoCandidate,
    /// No candidate cell has a value for every factor.
    NoSuitableCell,
}

/// A facility added.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewFacility {
    /// Iteration that added it (AccessMod 5 `amId`).
    pub id: u32,
    /// `facility_<id>`.
    pub name: String,
    /// Capacity class.
    pub label: String,
    /// Capacity.
    pub capacity: f64,
    /// Suitability of its cell.
    pub suitability: f64,
    /// Grid row.
    pub row: usize,
    /// Grid column.
    pub col: usize,
}

/// Result counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ScalingUpSummary {
    /// Iterations run.
    pub iterations: u32,
    /// Facilities added.
    pub facilities_added: usize,
    /// Candidate cells evaluated (searches).
    pub candidates_evaluated: u64,
    /// Why it stopped.
    pub stop: Option<StopReason>,
    /// Population of `population`.
    pub population_total: f64,
    /// Residual population off barriers at start.
    pub population: f64,
    /// Residual population at the end.
    pub population_residual: f64,
    /// Population covered, percent of `population_total`.
    pub population_covered_percent: f64,
}

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct ScalingUpRecord {
    /// Job with resolved paths and defaults.
    pub job: ScalingUpJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// DEM grid.
    pub grid: GridReport,
    /// Input counts.
    pub input_summary: InputSummary,
    /// Totals.
    pub result: ScalingUpSummary,
    /// Facilities added.
    pub facilities: Vec<NewFacility>,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Execution details.
#[derive(Debug, Clone, Serialize)]
pub struct ScalingUpStores {
    /// Cache counters of the input grid.
    pub input_store: StoreStats,
    /// Cache counters of the residual population grid.
    pub residual_store: StoreStats,
}

/// `run.json` of a scaling up.
pub type ScalingUpReport = Report<ScalingUpRecord, ScalingUpStores>;

/// Deviations from AccessMod 5 that apply to every run.
pub fn deviations() -> Vec<Deviation> {
    vec![
        Deviation {
            code: "dem_nodata_barrier",
            message: "DEM NoData cells are barriers (see travel time).",
        },
        Deviation {
            code: "scaling_up_limits",
            message: "max_coverage_percent and max_facilities are used as given. AccessMod 5 \
                      treats a coverage limit above 99 % as 100 % and a facility limit \
                      above 10^6 as 10^6.",
        },
    ]
}

/// How a factor reads its values.
enum Values {
    /// Its own grid.
    Grid(Box<TiledGrid<f64>>),
    /// The residual population, missing values as 0.
    ResidualZeroed,
    /// The facility distances.
    FacilityDistance,
}

/// A factor ready for an iteration.
struct FactorState {
    factor: Factor,
    values: Values,
    /// Integer raster: integer arithmetic in the rescale.
    integer: bool,
    /// The layer has no cell: the factor is skipped (AccessMod 5).
    empty: bool,
    /// Missing values take the maximum (travel time).
    missing_as_max: bool,
}

/// Mutable grids of a run.
struct Grids {
    residual: TiledGrid<f64>,
    candidates: TiledGrid<u8>,
    facility_distance: Option<TiledGrid<f64>>,
}

impl FactorState {
    fn value(&mut self, grids: &mut Grids, row: usize, col: usize) -> Result<f64> {
        Ok(match &mut self.values {
            Values::Grid(grid) => grid.get(row, col)?,
            Values::ResidualZeroed => {
                let v = grids.residual.get(row, col)?;
                if v.is_nan() { 0.0 } else { v }
            }
            Values::FacilityDistance => {
                let grid = grids
                    .facility_distance
                    .as_mut()
                    .expect("facility distances kept when used");
                grid.get(row, col)?
            }
        })
    }
}

fn check_cancel(cancelled: &dyn Fn() -> bool) -> impl FnMut() -> Result<()> + '_ {
    move || {
        if cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Values of a raster into `out` (NaN without value): whether its band holds
/// integers, and whether any cell has a value.
fn read_raster(
    path: &Path,
    grid: &GridSpec,
    out: &mut TiledGrid<f64>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(bool, bool)> {
    let mut any = false;
    let (dataset, alignment) = open_aligned_raster(path, grid, "layer")?;
    let integer = dataset
        .rasterband(1)
        .map_err(Error::gdal(path))?
        .band_type()
        .is_integer();
    let band = AlignedBand::new(&dataset, path, alignment, grid)?;
    for_each_block(grid, (BLOCK, BLOCK), &[&band], None, cancelled, |block| {
        let width = block.cols.len();
        for i in 0..block.len() {
            let (row, col) = (block.rows.start + i / width, block.cols.start + i % width);
            let value = block.value(0, i);
            any |= value.is_some();
            out.set(row, col, value.unwrap_or(f64::NAN))?;
        }
        Ok(())
    })?;
    Ok((integer, any))
}

/// Mark the cells of a raster or vector layer as sources (0.0) in `out`
/// (filled with infinity); `false` when the layer has no cell.
fn mark_sources(
    layer: &Layer,
    grid: &GridSpec,
    out: &mut TiledGrid<f64>,
    cancelled: &dyn Fn() -> bool,
) -> Result<bool> {
    fill(out, f64::INFINITY, cancelled)?;
    match layer {
        Layer::Raster(path) => {
            read_raster(path, grid, out, cancelled)?;
            let mut any = false;
            map_cells(out, cancelled, |v| {
                if v.is_nan() {
                    f64::INFINITY
                } else {
                    any = true;
                    0.0
                }
            })?;
            Ok(any)
        }
        Layer::Vector(source) => {
            let vector = open_vector(source, grid)?;
            let counts = burn_layer(&vector, grid, out, Burn::Thin, cancelled, |_| {
                Ok(Some(|c: &mut f64| *c = 0.0))
            })?;
            Ok(counts.used > 0)
        }
        Layer::Facilities | Layer::Population => unreachable!("dynamic layers are not marked"),
    }
}

fn fill(grid: &mut TiledGrid<f64>, value: f64, cancelled: &dyn Fn() -> bool) -> Result<()> {
    map_cells(grid, cancelled, |_| value)
}

/// Replace every cell `v` by `f(v)`, row by row.
fn map_cells(
    grid: &mut TiledGrid<f64>,
    cancelled: &dyn Fn() -> bool,
    mut f: impl FnMut(f64) -> f64,
) -> Result<()> {
    let geometry = grid.geometry();
    let mut row_values = vec![0.0; geometry.cols()];
    for row in 0..geometry.rows() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        grid.read_row(row, 0, &mut row_values)?;
        for v in &mut row_values {
            *v = f(*v);
        }
        grid.write_row(row, 0, &row_values)?;
    }
    Ok(())
}

/// Source cells (value 0.0) of a marked grid, at most `max` (the solver
/// queue holds them all at start: the plan reserves memory for its queue).
fn source_cells(
    grid: &mut TiledGrid<f64>,
    max: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Facility>> {
    let mut cells = Vec::new();
    grid.for_each_row_piece::<Error>(BLOCK * 16, |row, first, values| {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        for (k, v) in values.iter().enumerate() {
            if *v == 0.0 {
                if cells.len() == max {
                    return Err(Error::invalid(format!(
                        "a travel-time layer starts from more than {max} cells: \
                         raise --memory or simplify the layer"
                    )));
                }
                cells.push(Facility {
                    row,
                    col: first + k,
                });
            }
        }
        Ok(())
    })?;
    Ok(cells)
}

/// Travel time from `sources`, whole minutes rounded up (AccessMod 5
/// `amCleanTravelTime`), unlimited up to 32 767 minutes; NaN elsewhere.
/// Sources on barriers are skipped.
#[allow(clippy::too_many_arguments)] // reason: one private helper, explicit inputs
fn travel_minutes(
    inputs: &mut TiledGrid<InputCell>,
    scenario: &[CellMobility],
    sources: &[Facility],
    config: &SolverConfig,
    out: &mut TiledGrid<f64>,
    state_tiles: usize,
    tmp_dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    fill(out, f64::NAN, cancelled)?;
    // Sources on barriers start nothing (r.cost skips them).
    let mut usable = Vec::with_capacity(sources.len());
    for (k, s) in sources.iter().enumerate() {
        if k % 65_536 == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        if inputs.get(s.row, s.col)?.is_passable() {
            usable.push(*s);
        }
    }
    let sources = usable.as_slice();
    if sources.is_empty() {
        return Ok(());
    }
    let mut state = TiledGrid::new(
        inputs.geometry(),
        SolverCell::UNREACHED,
        state_tiles.max(1),
        tmp_dir,
        "scaling_up",
    )?;
    let mut failure = None;
    solve_until(
        inputs,
        scenario,
        sources,
        config,
        &mut state,
        cancelled,
        &mut |r, c, cell| {
            let minutes = (cell.time_s / 60.0).ceil();
            if minutes <= MAX_MINUTES
                && let Err(e) = out.set(r, c, minutes)
            {
                failure = Some(e);
                return Flow::Stop;
            }
            Flow::Continue
        },
    )?;
    match failure {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

/// Keep or drop candidates by their distance to a layer (`distance` grid,
/// metres; sources at 0).
fn apply_exclusion(
    candidates: &mut TiledGrid<u8>,
    distance: &mut TiledGrid<f64>,
    buffer_m: f64,
    keep: Keep,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let geometry = candidates.geometry();
    let mut flags = vec![0u8; geometry.cols()];
    let mut d = vec![0.0; geometry.cols()];
    for row in 0..geometry.rows() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        candidates.read_row(row, 0, &mut flags)?;
        distance.read_row(row, 0, &mut d)?;
        for (flag, d) in flags.iter_mut().zip(&d) {
            let inside = *d <= buffer_m;
            if inside != (keep == Keep::Inside) {
                *flag = 0;
            }
        }
        candidates.write_row(row, 0, &flags)?;
    }
    Ok(())
}

/// Drop the candidates whose centre is within `radius_m` of cell `(r, c)`.
fn exclude_near(
    candidates: &mut TiledGrid<u8>,
    (r, c): (usize, usize),
    radius_m: f64,
    grid: &GridSpec,
) -> Result<()> {
    let (ew, ns) = (grid.cell.ew_m(), grid.cell.ns_m());
    let (kc, kr) = (
        (radius_m / ew).floor() as isize,
        (radius_m / ns).floor() as isize,
    );
    for dr in -kr..=kr {
        for dc in -kc..=kc {
            let (row, col) = (r as isize + dr, c as isize + dc);
            let (a, b) = (ew * dc as f64, ns * dr as f64);
            if row >= 0
                && col >= 0
                && (row as usize) < grid.rows
                && (col as usize) < grid.cols
                && a * a + b * b <= radius_m * radius_m
            {
                candidates.set(row as usize, col as usize, 0)?;
            }
        }
    }
    Ok(())
}

/// Lower the facility distances with a new facility at `(fr, fc)`, with
/// the arithmetic of the distance transform.
fn add_facility_distance(
    distance: &mut TiledGrid<f64>,
    (fr, fc): (usize, usize),
    grid: &GridSpec,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let (ew, ns) = (grid.cell.ew_m(), grid.cell.ns_m());
    let geometry = distance.geometry();
    let mut row_values = vec![0.0; geometry.cols()];
    for row in 0..geometry.rows() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        distance.read_row(row, 0, &mut row_values)?;
        let b = ns * (row as f64 - fr as f64);
        for (col, v) in row_values.iter_mut().enumerate() {
            let a = ew * (col as f64 - fc as f64);
            *v = v.min((a * a + b * b).sqrt());
        }
        distance.write_row(row, 0, &row_values)?;
    }
    Ok(())
}

/// Suitability of every candidate cell, row-major: `visit(row, col, s)`.
fn for_each_suitability(
    factors: &mut [FactorState],
    rescales: &[Option<Rescale>],
    weight_sum: f64,
    grids: &mut Grids,
    cancelled: &dyn Fn() -> bool,
    mut visit: impl FnMut(usize, usize, f64) -> Result<()>,
) -> Result<()> {
    let geometry = grids.candidates.geometry();
    let mut flags = vec![0u8; geometry.cols()];
    let mut weighted = Vec::with_capacity(factors.len());
    for row in 0..geometry.rows() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        grids.candidates.read_row(row, 0, &mut flags)?;
        for (col, flag) in flags.iter().enumerate() {
            if *flag == 0 {
                continue;
            }
            weighted.clear();
            let mut missing = false;
            for (state, rescale) in factors.iter_mut().zip(rescales) {
                let Some(rescale) = rescale else { continue };
                let mut v = state.value(grids, row, col)?;
                if v.is_nan() {
                    if state.missing_as_max {
                        v = rescale.max_value();
                    } else {
                        missing = true;
                        break;
                    }
                }
                weighted.push(rescale.apply(v));
            }
            if !missing {
                visit(row, col, suitability(weighted.iter().copied(), weight_sum))?;
            }
        }
    }
    Ok(())
}

/// Compute the scaling up and write every output atomically.
pub fn run(
    prepared: PreparedScalingUp,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<(PathBuf, ScalingUpReport)> {
    let cancelled = monitor.cancelled();
    let PreparedScalingUp {
        job,
        grid,
        mut inputs,
        scenario,
        loaded,
        summary: input_summary,
        warnings,
        plan,
        fingerprints,
        mut durations_s,
    } = prepared;
    let staging = Staging::create(&job.output_dir)?;
    let geometry = inputs.geometry();
    let tiles = plan.resident_tiles;
    let tmp = &settings.tmp_dir;
    let new_f64 = |fill: f64, name: &'static str| TiledGrid::new(geometry, fill, tiles, tmp, name);

    // Residual population and totals.
    monitor.stage("setup");
    let started = Instant::now();
    let mut residual = new_f64(f64::NAN, "residual")?;
    let residual_path = job.population_residual.as_ref().unwrap_or(&job.population);
    let (residual_ds, alignment) = open_aligned_raster(residual_path, &grid, "population")?;
    let residual_band = AlignedBand::new(&residual_ds, residual_path, alignment, &grid)?;
    let PopulationTotals {
        total: residual_total,
        off_barriers: total,
        ..
    } = init_residual(
        &grid,
        &residual_band,
        &mut inputs,
        &mut residual,
        None,
        cancelled,
    )?;
    let population_total = if job.population_residual.is_some() {
        let (ds, alignment) = open_aligned_raster(&job.population, &grid, "population")?;
        let band = AlignedBand::new(&ds, &job.population, alignment, &grid)?;
        let mut sum = 0.0;
        for_each_block(&grid, (BLOCK, BLOCK), &[&band], None, cancelled, |block| {
            sum += (0..block.len())
                .filter_map(|i| block.value(0, i))
                .sum::<f64>();
            Ok(())
        })?;
        sum
    } else {
        residual_total
    };

    // Candidates: every cell that can be entered.
    let mut candidates = TiledGrid::new(geometry, 0u8, tiles, tmp, "candidates")?;
    {
        let mut flags = vec![0u8; geometry.cols()];
        for row in 0..geometry.rows() {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            for (col, flag) in flags.iter_mut().enumerate() {
                *flag = u8::from(inputs.get(row, col)?.is_passable());
            }
            candidates.write_row(row, 0, &flags)?;
        }
    }

    // Facilities and their distances.
    let mut facility_cells: Vec<(usize, usize)> = loaded
        .as_ref()
        .map(|l| l.kept.iter().map(|f| (f.row, f.col)).collect())
        .unwrap_or_default();
    let (ew, ns) = (grid.cell.ew_m(), grid.cell.ns_m());
    let facility_distance = if job.uses_facilities() {
        let mut d = new_f64(f64::INFINITY, "facility_distance")?;
        for &(r, c) in &facility_cells {
            d.set(r, c, 0.0)?;
        }
        distance_to_sources(&mut d, ew, ns, check_cancel(cancelled))?;
        Some(d)
    } else {
        None
    };
    let mut grids = Grids {
        residual,
        candidates,
        facility_distance,
    };

    // Static exclusions, once.
    let mut scratch = new_f64(f64::INFINITY, "scratch")?;
    for exclusion in &job.exclusions {
        if exclusion.layer == Layer::Facilities {
            continue;
        }
        if mark_sources(&exclusion.layer, &grid, &mut scratch, cancelled)? {
            if exclusion.buffer_m > 0.0 {
                distance_to_sources(&mut scratch, ew, ns, check_cancel(cancelled))?;
            }
            apply_exclusion(
                &mut grids.candidates,
                &mut scratch,
                exclusion.buffer_m,
                exclusion.keep,
                cancelled,
            )?;
        }
    }

    // Factors: static grids computed once.
    // The solver queue starts with every source cell: they share its reserve.
    let max_sources =
        (plan.queue_reserve_bytes as usize / (4 * std::mem::size_of::<Facility>())).max(1);
    let mut factors = Vec::with_capacity(job.suitability.len());
    for factor in job.active_factors() {
        let mut state = FactorState {
            factor: factor.clone(),
            values: Values::ResidualZeroed,
            integer: false,
            empty: false,
            missing_as_max: matches!(factor, Factor::TravelTime { .. }),
        };
        match factor {
            Factor::Population {
                layer, window_m, ..
            } => {
                let size = window_cells(*window_m, ns);
                if let Layer::Raster(path) = layer {
                    state.empty = !read_raster(path, &grid, &mut scratch, cancelled)?.1;
                    let mut out = new_f64(f64::NAN, "factor")?;
                    if size > 0 {
                        circular_sum(&mut scratch, &mut out, size / 2, check_cancel(cancelled))?;
                    } else {
                        let mut values = vec![0.0; geometry.cols()];
                        for row in 0..geometry.rows() {
                            if cancelled() {
                                return Err(Error::Cancelled);
                            }
                            scratch.read_row(row, 0, &mut values)?;
                            for v in &mut values {
                                if v.is_nan() {
                                    *v = 0.0;
                                }
                            }
                            out.write_row(row, 0, &values)?;
                        }
                    }
                    state.values = Values::Grid(Box::new(out));
                } else if size > 0 {
                    state.values = Values::Grid(Box::new(new_f64(f64::NAN, "factor")?));
                }
            }
            Factor::Distance { layer, .. } => {
                if *layer == Layer::Facilities {
                    state.values = Values::FacilityDistance;
                } else {
                    let mut out = new_f64(f64::INFINITY, "factor")?;
                    state.empty = !mark_sources(layer, &grid, &mut out, cancelled)?;
                    distance_to_sources(&mut out, ew, ns, check_cancel(cancelled))?;
                    state.values = Values::Grid(Box::new(out));
                }
            }
            Factor::TravelTime {
                layer,
                method: name,
                direction,
                neighbourhood,
                ..
            } => {
                state.integer = true;
                let mut out = new_f64(f64::NAN, "factor")?;
                if *layer != Layer::Facilities {
                    state.empty = !mark_sources(layer, &grid, &mut scratch, cancelled)?;
                    let sources = source_cells(&mut scratch, max_sources, cancelled)?;
                    let config = SolverConfig {
                        method: method(*name, job.mobility_model),
                        direction: *direction,
                        neighbourhood: *neighbourhood,
                        cell: grid.cell,
                        limit_s: None,
                    };
                    travel_minutes(
                        &mut inputs,
                        &scenario,
                        &sources,
                        &config,
                        &mut out,
                        tiles,
                        tmp,
                        cancelled,
                    )?;
                }
                state.values = Values::Grid(Box::new(out));
            }
            Factor::Priority { layer, .. } => {
                let Layer::Raster(path) = layer else {
                    unreachable!("checked by the job")
                };
                let mut out = new_f64(f64::NAN, "factor")?;
                let (integer, any) = read_raster(path, &grid, &mut out, cancelled)?;
                (state.integer, state.empty) = (integer, !any);
                state.values = Values::Grid(Box::new(out));
            }
        }
        factors.push(state);
    }
    drop(scratch);
    durations_s.insert("setup", started.elapsed().as_secs_f64());

    // Iterations.
    monitor.stage("iterations");
    let started = Instant::now();
    let config = job.solver_config(&grid);
    let limit_band = (job.limit_s / job.band_s).ceil() as u32;
    let mut spool = CellSpool::new(tmp)?;
    let mut best_cells = CellSpool::new(tmp)?;
    let mut catchments = if job.catchments {
        Some(FeatureWriter::create(
            &staging.path().join(CATCHMENTS_FILE),
            "catchments",
            &grid.srs,
            OGRwkbGeometryType::wkbMultiPolygon,
            &[
                ("order", OGRFieldType::OFTInteger),
                ("facility_id", OGRFieldType::OFTInteger),
                ("name", OGRFieldType::OFTString),
                ("case", OGRFieldType::OFTString),
            ],
        )?)
    } else {
        None
    };
    let mut remaining = total;
    let mut result = ScalingUpSummary {
        population_total,
        population: total,
        ..ScalingUpSummary::default()
    };
    let mut added: Vec<NewFacility> = Vec::new();
    let mut rows = Vec::new();
    let covered_percent = |remaining: f64| {
        if population_total > 0.0 {
            (population_total - remaining) / population_total * 100.0
        } else {
            0.0
        }
    };
    // Time spent reporting progress does not count towards `max_duration_s`:
    // a slow progress sink must not change which facilities are added.
    let mut reporting = std::time::Duration::ZERO;
    for iteration in 1..=job.max_facilities {
        let report_started = Instant::now();
        monitor.advance(
            "iterations",
            u64::from(iteration - 1),
            Some(u64::from(job.max_facilities)),
        );
        reporting += report_started.elapsed();
        if covered_percent(remaining) >= job.max_coverage_percent {
            result.stop = Some(StopReason::Coverage);
            break;
        }
        if job
            .max_duration_s
            .is_some_and(|d| started.elapsed().saturating_sub(reporting).as_secs_f64() >= d)
        {
            result.stop = Some(StopReason::Duration);
            break;
        }
        result.iterations = iteration;

        // Exclusions around facilities.
        for exclusion in &job.exclusions {
            if exclusion.layer == Layer::Facilities && !facility_cells.is_empty() {
                let distance = grids
                    .facility_distance
                    .as_mut()
                    .expect("facility distances kept when used");
                apply_exclusion(
                    &mut grids.candidates,
                    distance,
                    exclusion.buffer_m,
                    exclusion.keep,
                    cancelled,
                )?;
            }
        }
        let mut count = 0u64;
        grids
            .candidates
            .for_each_row_piece::<Error>(BLOCK * 16, |_, _, flags| {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                count += flags.iter().filter(|f| **f != 0).count() as u64;
                Ok(())
            })?;
        if count == 0 {
            result.stop = Some(StopReason::NoCandidate);
            break;
        }

        // Dynamic factor values.
        for state in &mut factors {
            match (&state.factor, &mut state.values) {
                (
                    Factor::Population {
                        layer: Layer::Population,
                        window_m,
                        ..
                    },
                    Values::Grid(out),
                ) => {
                    let size = window_cells(*window_m, ns);
                    circular_sum(&mut grids.residual, out, size / 2, check_cancel(cancelled))?;
                }
                (
                    Factor::Distance {
                        layer: Layer::Facilities,
                        ..
                    },
                    _,
                ) => {
                    state.empty = facility_cells.is_empty();
                }
                (
                    Factor::TravelTime {
                        layer: Layer::Facilities,
                        method: name,
                        direction,
                        neighbourhood,
                        ..
                    },
                    Values::Grid(out),
                ) => {
                    state.empty = facility_cells.is_empty();
                    let sources: Vec<Facility> = facility_cells
                        .iter()
                        .map(|&(row, col)| Facility { row, col })
                        .collect();
                    let config = SolverConfig {
                        method: method(*name, job.mobility_model),
                        direction: *direction,
                        neighbourhood: *neighbourhood,
                        cell: grid.cell,
                        limit_s: None,
                    };
                    travel_minutes(
                        &mut inputs,
                        &scenario,
                        &sources,
                        &config,
                        out,
                        tiles,
                        tmp,
                        cancelled,
                    )?;
                }
                _ => {}
            }
        }

        // Rescales over the candidates.
        let used: Vec<bool> = factors
            .iter()
            .map(|s| !s.empty && s.factor.weight() > 0.0)
            .collect();
        let mut bounds: Vec<Option<(f64, f64)>> = vec![None; factors.len()];
        {
            let mut flags = vec![0u8; geometry.cols()];
            for row in 0..geometry.rows() {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                grids.candidates.read_row(row, 0, &mut flags)?;
                for (col, flag) in flags.iter().enumerate() {
                    if *flag == 0 {
                        continue;
                    }
                    for (k, state) in factors.iter_mut().enumerate() {
                        if !used[k] {
                            continue;
                        }
                        let v = state.value(&mut grids, row, col)?;
                        if !v.is_nan() {
                            bounds[k] =
                                Some(bounds[k].map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
                        }
                    }
                }
            }
        }
        let rescales: Vec<Option<Rescale>> = factors
            .iter()
            .zip(&bounds)
            .zip(&used)
            .map(|((state, bounds), used)| {
                let (lo, hi) = (*bounds)?;
                used.then(|| {
                    Rescale::new(
                        lo,
                        hi,
                        state.integer,
                        state.factor.prefer() == Prefer::Low,
                        state.factor.weight(),
                    )
                })
            })
            .collect();
        let weight_sum: f64 = factors
            .iter()
            .zip(&used)
            .filter(|(_, u)| **u)
            .map(|(s, _)| s.factor.weight())
            .sum();
        // A factor used without any value on the candidates leaves no cell
        // suitable.
        let mut best_suitability: Option<f64> = None;
        if used.iter().zip(&rescales).all(|(u, r)| !u || r.is_some()) {
            for_each_suitability(
                &mut factors,
                &rescales,
                weight_sum,
                &mut grids,
                cancelled,
                |_, _, s| {
                    best_suitability = Some(best_suitability.map_or(s, |b: f64| b.max(s)));
                    Ok(())
                },
            )?;
        }
        let Some(max_suitability) = best_suitability else {
            result.stop = Some(StopReason::NoSuitableCell);
            break;
        };

        // Evaluate the most suitable cells.
        let mut best: Option<(usize, usize, f64)> = None;
        let mut evaluated = 0u64;
        // They can be many (ties): spooled to disk, then searched in turn.
        best_cells.clear()?;
        let cols = geometry.cols() as u64;
        for_each_suitability(
            &mut factors,
            &rescales,
            weight_sum,
            &mut grids,
            cancelled,
            |row, col, s| {
                if s >= max_suitability {
                    best_cells
                        .push(row as u64 * cols + col as u64, 0)
                        .map_err(Error::file(best_cells.path()))?;
                }
                Ok(())
            },
        )?;
        best_cells.for_each(cancelled, |cell, _| {
            let (row, col) = ((cell / cols) as usize, (cell % cols) as usize);
            let bands = search_bands(
                &mut inputs,
                &scenario,
                Facility { row, col },
                &config,
                job.band_s,
                &mut grids.residual,
                &mut spool,
                tiles,
                tmp,
                cancelled,
            )?;
            evaluated += 1;
            let population: f64 = bands.iter().map(|b| b.population).sum();
            if best.is_none_or(|(_, _, p)| population > p) {
                best = Some((row, col, population));
            }
            Ok(())
        })?;
        result.candidates_evaluated += evaluated;
        let (row, col, population) = best.expect("at least one cell of maximum suitability");
        if population < 1.0 {
            // No one reached: the cells within 1 m of it (r.buffer around the
            // point) are no candidates any more.
            exclude_near(&mut grids.candidates, (row, col), 1.0, &grid)?;
            continue;
        }
        let classes: Vec<CapacityClass> = job
            .capacities
            .iter()
            .map(|c| CapacityClass {
                min: c.min,
                max: c.max,
                capacity: c.capacity,
                label: c.label.clone(),
            })
            .collect();
        let class = &classes[capacity_class(population, &classes).expect("capacities checked")];

        // The new facility takes its population.
        let bands = search_bands(
            &mut inputs,
            &scenario,
            Facility { row, col },
            &config,
            job.band_s,
            &mut grids.residual,
            &mut spool,
            tiles,
            tmp,
            cancelled,
        )?;
        let caught = catchment(&bands, Some(class.capacity), limit_band);
        let facility = NewFacility {
            id: iteration,
            name: format!("facility_{iteration}"),
            label: class.label.clone(),
            capacity: class.capacity,
            suitability: max_suitability,
            row,
            col,
        };
        let order = added.len() + 1;
        if let (Some(writer), Some(limit)) = (catchments.as_mut(), caught.band_limit)
            && let Some(polygon) = catchment_polygon(&mut spool, limit, &grid, tmp, cancelled)?
        {
            writer.write(
                polygon,
                &[
                    Some(FieldValue::IntegerValue(order as i32)),
                    Some(FieldValue::IntegerValue(iteration as i32)),
                    Some(FieldValue::StringValue(facility.name.clone())),
                    caught
                        .case
                        .map(|c| FieldValue::StringValue(format!("{c:?}"))),
                ],
            )?;
        }
        let before = remaining;
        remaining -= remove_covered(&mut spool, &mut grids.residual, &caught, cancelled)?;
        let step = ResidualStep {
            before,
            after: remaining,
            covered_percent: if total > 0.0 {
                (total - remaining) / total * 100.0
            } else {
                0.0
            },
        };
        rows.push((order, facility.clone(), caught, step));
        facility_cells.push((row, col));
        if let Some(distance) = grids.facility_distance.as_mut() {
            add_facility_distance(distance, (row, col), &grid, cancelled)?;
        }
        added.push(facility);
    }
    if result.stop.is_none() {
        result.stop = Some(StopReason::MaxFacilities);
    }
    if let Some(writer) = catchments {
        writer.close()?;
    }
    durations_s.insert("iterations", started.elapsed().as_secs_f64());

    // Outputs.
    monitor.stage("write_outputs");
    let started = Instant::now();
    write_residual(
        &staging.path().join(RESIDUAL_FILE),
        &grid,
        &mut grids.residual,
        settings,
        cancelled,
    )?;
    write_table(&staging.path().join(CAPACITY_FILE), &job, &rows)?;
    write_facilities(
        &staging.path().join(FACILITIES_FILE),
        &grid,
        loaded.as_ref(),
        &rows,
    )?;
    durations_s.insert("write_outputs", started.elapsed().as_secs_f64());

    result.facilities_added = added.len();
    result.population_residual = remaining;
    result.population_covered_percent = covered_percent(remaining);
    let stores = ScalingUpStores {
        input_store: inputs.stats(),
        residual_store: grids.residual.stats(),
    };
    drop(spool);
    drop(best_cells);
    let report = Report::new(
        ScalingUpRecord {
            job,
            inputs: fingerprints,
            grid: GridReport::of(&grid),
            input_summary,
            result,
            facilities: added,
            warnings,
            deviations: deviations(),
        },
        Execution::now(settings, plan, durations_s, stores),
    );
    let output_dir = output::finish(staging, &report.run.job, &report, cancelled)?;
    Ok((output_dir, report))
}

fn write_residual(
    path: &Path,
    grid: &GridSpec,
    residual: &mut TiledGrid<f64>,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let tif = GeoTiff::create::<f64>(path, grid, 3, settings.threads, FLOAT_NODATA)?;
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
    tif.close()
}

type Row = (
    usize,
    NewFacility,
    accessmod_core::capacity::Catchment,
    ResidualStep,
);

fn write_table(path: &Path, job: &ScalingUpJob, rows: &[Row]) -> Result<()> {
    let mut csv = CsvWriter::create(
        path,
        &format!("order,facility_id,name,label,capacity,suitability,{CATCHMENT_COLUMNS}"),
    )?;
    for (order, f, c, step) in rows {
        let mut record = vec![
            order.to_string(),
            f.id.to_string(),
            f.name.clone(),
            csv_field(&serde_json::Value::String(f.label.clone())),
            table_number(Some(f.capacity)),
            table_number(Some(f.suitability)),
        ];
        record.extend(catchment_fields(c, job.limit_s, job.band_s, *step));
        csv.record(&record)?;
    }
    csv.finish()
}

/// Existing facilities (at their points, `added` 0) then the new ones (at
/// cell centres, `added` 1), as the AccessMod 5 output facility layer.
fn write_facilities(
    path: &Path,
    grid: &GridSpec,
    existing: Option<&LoadedFacilities>,
    rows: &[Row],
) -> Result<()> {
    let mut writer = FeatureWriter::create(
        path,
        "facilities",
        &grid.srs,
        OGRwkbGeometryType::wkbPoint,
        &[
            ("added", OGRFieldType::OFTInteger),
            ("order", OGRFieldType::OFTInteger),
            ("facility_id", OGRFieldType::OFTString),
            ("name", OGRFieldType::OFTString),
            ("label", OGRFieldType::OFTString),
            ("capacity", OGRFieldType::OFTReal),
            ("suitability", OGRFieldType::OFTReal),
            ("population_within_limit", OGRFieldType::OFTReal),
        ],
    )?;
    let point =
        |x: f64, y: f64| Geometry::from_wkt(&format!("POINT ({x} {y})")).map_err(Error::gdal(path));
    for record in existing.map(|l| l.kept.as_slice()).unwrap_or_default() {
        writer.write(
            point(record.x, record.y)?,
            &[
                Some(FieldValue::IntegerValue(0)),
                None,
                Some(FieldValue::StringValue(csv_field(&record.id))),
                None,
                None,
                None,
                None,
                None,
            ],
        )?;
    }
    for (order, f, c, _) in rows {
        let (x, y) = grid.cell_centre(f.row, f.col);
        writer.write(
            point(x, y)?,
            &[
                Some(FieldValue::IntegerValue(1)),
                Some(FieldValue::IntegerValue(*order as i32)),
                Some(FieldValue::StringValue(f.id.to_string())),
                Some(FieldValue::StringValue(f.name.clone())),
                Some(FieldValue::StringValue(f.label.clone())),
                Some(FieldValue::RealValue(f.capacity)),
                Some(FieldValue::RealValue(f.suitability)),
                c.population_within_limit.map(FieldValue::RealValue),
            ],
        )?;
    }
    writer.close()
}

/// Scaling up.
#[derive(Debug)]
pub struct ScalingUp;

impl Analysis for ScalingUp {
    const NAME: &'static str = "scaling-up";
    const OUTPUTS: &'static [Output] = &[
        Output {
            file: CAPACITY_FILE,
            class: Some(DataClass::ScalingUpResult),
        },
        Output {
            file: FACILITIES_FILE,
            class: Some(DataClass::FacilityNew),
        },
        Output {
            file: RESIDUAL_FILE,
            class: Some(DataClass::PopulationResidual),
        },
        Output {
            file: CATCHMENTS_FILE,
            class: Some(DataClass::CatchmentNew),
        },
    ];
    type Job = ScalingUpJob;
    type Prepared = PreparedScalingUp;
    type Report = ScalingUpReport;

    fn prepare(
        job: ScalingUpJob,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<PreparedScalingUp> {
        prepare(job, settings, monitor)
    }

    fn lint(job: &Self::Job, context: &dyn LintContext) -> Result<Vec<Warning>> {
        let layers: Vec<_> = job.facilities.iter().map(|f| ("facilities", f)).collect();
        lint_landscape(
            &job.dem,
            &job.land_cover,
            &job.scenario,
            &layers,
            job.facility_issues,
            context,
        )
    }

    fn run(
        prepared: PreparedScalingUp,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<(PathBuf, ScalingUpReport)> {
        run(prepared, settings, monitor)
    }

    fn warnings(prepared: &PreparedScalingUp) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &PreparedScalingUp) -> String {
        format!(
            "scaling up: at most {} facilities on a {} x {} grid",
            prepared.job.max_facilities, prepared.grid.cols, prepared.grid.rows
        )
    }

    fn validation(prepared: &PreparedScalingUp) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "facilities_skipped": prepared.loaded.as_ref().map(|l| &l.issues),
            "input_summary": prepared.summary,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &ScalingUpReport) -> String {
        let r = &report.run.result;
        format!(
            "{} facilities added, {:.2} % of {} people covered",
            r.facilities_added, r.population_covered_percent, r.population_total
        )
    }
}
