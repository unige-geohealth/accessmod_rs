//! Optimization (schema version 1): rank existing facilities by the
//! population their catchments add to the coverage, as AccessMod 5 does
//! (`tools/R/amAnalysisOptimization.R`; rules in
//! `accessmod_core::optimization`).
//!
//! Catchments (from a geographic coverage) are read once into a disk
//! spool. Each catchment selected is subtracted (GEOS difference) from the
//! catchments it intersects, whose population is then extracted again with
//! exact cell coverage fractions (`vector::covered_sum`). Memory holds a few
//! geometries and one record per catchment.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::optimization::{Candidate, ZoneQuota, select};
use gdal::vector::{Envelope, Geometry, LayerAccess, OGRwkbGeometryType, geometry_type_flatten};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Monitor, Output};
use crate::class::DataClass;
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::{FacilitySource, JobFile, check_files, check_schema_version};
use crate::output::{self, CsvWriter, Staging, csv_field};
use crate::raster::{AlignedBand, open_aligned_raster};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::vector::{GeometrySpool, SpoolSlot, VectorLayer, covered_sum, json_attribute};
use crate::zonal::{Zone, ZoneLayer, open_zones, read_zones};

/// Ranked facilities file name.
pub const OPTIMIZATION_FILE: &str = "optimization.csv";

/// Spool bytes beyond twice the current geometries before compaction.
const SPOOL_SLACK: u64 = 64 << 20;

/// Optimization job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptimizationJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Population raster (people per cell); defines the grid.
    #[schemars(extend("x-accessmod-class" = ["rPopulation", "rPopulationResidual"]))]
    pub population: PathBuf,
    /// Catchment polygons (`catchments.fgb` of a geographic coverage);
    /// `id_field` holds the identifier of each catchment's facility.
    #[schemars(extend("x-accessmod-class" = ["vCatchment", "vCatchmentNew"]))]
    pub catchments: FacilitySource,
    /// Facilities to select (> 0).
    pub max_facilities: usize,
    /// Minimum number of facilities per zone; none when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_quota: Option<ZoneQuotaJob>,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

/// A minimum number of facilities per zone: each catchment belongs to the
/// zone holding its facility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZoneQuotaJob {
    /// Zones.
    #[schemars(extend("x-accessmod-class" = ["vZone"]))]
    pub zones: ZoneLayer,
    /// Facility points; `id_field` matches the catchments' identifiers.
    #[schemars(extend("x-accessmod-class" = ["vFacility"]))]
    pub facilities: FacilitySource,
    /// Facilities per zone (> 0); zones times this minimum must not exceed
    /// `max_facilities`.
    pub min_per_zone: usize,
}

impl JobFile for OptimizationJob {
    fn for_each_path(&mut self, f: &mut dyn FnMut(&mut PathBuf)) {
        f(&mut self.population);
        f(&mut self.catchments.path);
        f(&mut self.output_dir);
        if let Some(quota) = &mut self.zone_quota {
            f(&mut quota.zones.path);
            f(&mut quota.facilities.path);
        }
    }
}

impl OptimizationJob {
    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues: Vec<String> = check_schema_version(self.schema_version)
            .into_iter()
            .collect();
        if self.catchments.id_field.trim().is_empty() {
            issues.push("catchments.id_field is empty".into());
        }
        if self.max_facilities == 0 {
            issues.push("max_facilities must be > 0".into());
        }
        if let Some(quota) = &self.zone_quota {
            if quota.zones.id_field.trim().is_empty() {
                issues.push("zone_quota.zones.id_field is empty".into());
            }
            if quota.facilities.id_field.trim().is_empty() {
                issues.push("zone_quota.facilities.id_field is empty".into());
            }
            if quota.min_per_zone == 0 {
                issues.push("zone_quota.min_per_zone must be > 0".into());
            }
        }
        issues
    }

    fn inputs(&self) -> Vec<(&'static str, &Path)> {
        let mut inputs = vec![
            ("population", self.population.as_path()),
            ("catchments", self.catchments.path.as_path()),
        ];
        if let Some(quota) = &self.zone_quota {
            inputs.push(("zones", quota.zones.path.as_path()));
            inputs.push(("facilities", quota.facilities.path.as_path()));
        }
        inputs
    }
}

/// A catchment read from the layer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatchmentRecord {
    /// Identifier of its facility.
    pub id: serde_json::Value,
    /// Zone of its facility (index in [`PreparedOptimization::zones`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<usize>,
}

/// Inputs validated, ready to compute.
#[derive(Debug)]
pub struct PreparedOptimization {
    /// Job with resolved paths.
    pub job: OptimizationJob,
    /// Grid of the population raster.
    pub grid: GridSpec,
    /// Catchments in layer order.
    pub catchments: Vec<CatchmentRecord>,
    /// Zones (with a quota).
    pub zones: Vec<Zone>,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Input files.
    pub fingerprints: Vec<Fingerprint>,
    /// Seconds spent per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
}

/// Identifier as text, to match catchments with facilities whatever the
/// field types (`4`, `4.0` and `"4"` match, as AccessMod 5 compares text).
fn id_key(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => match n.as_f64() {
            Some(v) if v.fract() == 0.0 && v.abs() < 1e15 => format!("{}", v as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// Text of a validation error, for a list of issues.
fn issue(error: Error) -> String {
    match error {
        Error::Validation { issues } => issues.join("; "),
        other => other.to_string(),
    }
}

fn open_layer(source: &FacilitySource, grid: &GridSpec, role: &str) -> Result<VectorLayer> {
    VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        role,
    )
}

/// Sizes found while scanning the catchments.
#[derive(Debug, Default)]
struct Scan {
    ids: Vec<serde_json::Value>,
    id_bytes: usize,
    largest_wkb: u64,
    total_wkb: u64,
}

/// Identifiers and geometry sizes of the catchments; every geometry is
/// checked (present, reprojectable).
fn scan_catchments(
    vector: &VectorLayer,
    source: &FacilitySource,
    cancelled: &dyn Fn() -> bool,
) -> Result<Scan> {
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let id_index = vector.field_index(&layer, &source.id_field)?;
    let mut scan = Scan::default();
    let mut issues = Vec::new();
    for (k, feature) in layer.features().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let problem = vector.feature_problem(feature.fid(), k as u64);
        let id = json_attribute(feature.field(id_index).map_err(Error::gdal(&source.path))?);
        let Some(id) = id else {
            issues.push(issue(problem(&format!("empty {:?}", source.id_field))));
            continue;
        };
        let size = match feature.geometry() {
            Some(geometry) => match vector.to_grid_geometry(geometry) {
                Ok(g) => g.wkb().map_err(Error::gdal(&source.path))?.len() as u64,
                Err(why) => {
                    issues.push(issue(problem(why)));
                    continue;
                }
            },
            None => {
                issues.push(issue(problem("has no geometry")));
                continue;
            }
        };
        scan.largest_wkb = scan.largest_wkb.max(size);
        scan.total_wkb += size;
        scan.id_bytes += id_key(&id).len();
        scan.ids.push(id);
    }
    if issues.is_empty() {
        Ok(scan)
    } else {
        Err(Error::Validation { issues })
    }
}

/// Zones of the facilities.
struct FacilityZones {
    /// Zone of each facility identifier key.
    zone_of: BTreeMap<String, Option<usize>>,
    /// Bytes of the keys.
    key_bytes: usize,
    /// Largest zone geometry, WKB bytes.
    largest_zone_wkb: u64,
}

/// Facility points by identifier key, in the grid CRS, and the largest
/// zone geometry; the zone of each identifier is the last zone, in layer
/// order, that intersects one of its points (as AccessMod 5).
fn facility_zones(
    quota: &ZoneQuotaJob,
    grid: &GridSpec,
    zone_index: &[u32],
    cancelled: &dyn Fn() -> bool,
) -> Result<FacilityZones> {
    let vector = open_layer(&quota.facilities, grid, "facilities")?;
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let id_index = vector.field_index(&layer, &quota.facilities.id_field)?;
    let mut points: Vec<(String, f64, f64)> = Vec::new();
    let mut issues = Vec::new();
    for (k, feature) in layer.features().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let problem = vector.feature_problem(feature.fid(), k as u64);
        let Some(id) = json_attribute(
            feature
                .field(id_index)
                .map_err(Error::gdal(&quota.facilities.path))?,
        ) else {
            issues.push(issue(problem(&format!(
                "empty {:?}",
                quota.facilities.id_field
            ))));
            continue;
        };
        let Some(geometry) = feature.geometry() else {
            issues.push(issue(problem("has no geometry")));
            continue;
        };
        if geometry_type_flatten(geometry.geometry_type()) != OGRwkbGeometryType::wkbPoint {
            issues.push(issue(problem("is not a point")));
            continue;
        }
        match vector.to_grid_geometry(geometry) {
            Ok(point) => {
                let (x, y, _) = point.get_point(0);
                points.push((id_key(&id), x, y));
            }
            Err(why) => issues.push(issue(problem(why))),
        }
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let mut zone_of: BTreeMap<String, Option<usize>> = points
        .iter()
        .map(|(key, _, _)| (key.clone(), None))
        .collect();
    let mut probe = Geometry::empty(OGRwkbGeometryType::wkbPoint)
        .map_err(Error::gdal(&quota.facilities.path))?;
    let zones = open_zones(&quota.zones, grid)?;
    let dataset = zones.dataset()?;
    let mut layer = zones.layer(&dataset)?;
    let mut largest = 0u64;
    for (k, feature) in layer.features().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let Some(geometry) = feature.geometry() else {
            continue;
        };
        let zone = zones
            .to_grid_geometry(geometry)
            .map_err(|why| zones.feature_problem(feature.fid(), k as u64)(why))?;
        largest = largest.max(zone.wkb().map_err(Error::gdal(&quota.zones.path))?.len() as u64);
        let envelope = zone.envelope();
        let index = zone_index[k] as usize - 1;
        for (n, (key, x, y)) in points.iter().enumerate() {
            if n % 4096 == 0 && cancelled() {
                return Err(Error::Cancelled);
            }
            let (x, y) = (*x, *y);
            let inside_box = x >= envelope.MinX
                && x <= envelope.MaxX
                && y >= envelope.MinY
                && y <= envelope.MaxY;
            if inside_box {
                probe.set_point_2d(0, (x, y));
                if zone.intersects(&probe) {
                    zone_of.insert(key.clone(), Some(index));
                }
            }
        }
    }
    let key_bytes = points.iter().map(|(key, _, _)| key.len()).sum();
    Ok(FacilityZones {
        zone_of,
        key_bytes,
        largest_zone_wkb: largest,
    })
}

/// What the memory plan counts: records held per catchment, facility and
/// zone (identifiers measured), and the largest geometries.
#[derive(Debug, Clone, Copy, Default)]
struct Sizes {
    catchments: usize,
    catchment_id_bytes: usize,
    facilities: usize,
    facility_id_bytes: usize,
    zones: usize,
    zone_text_bytes: usize,
    largest_wkb: u64,
    total_wkb: u64,
}

impl Sizes {
    /// Bytes per identifier assumed before any is read.
    const ID_GUESS: usize = 32;

    /// Sizes from feature counts alone, before reading identifiers.
    fn guess(catchments: usize, facilities: usize, zones: usize) -> Self {
        Self {
            catchments,
            catchment_id_bytes: catchments * Self::ID_GUESS,
            facilities,
            facility_id_bytes: facilities * Self::ID_GUESS,
            zones,
            zone_text_bytes: zones * 2 * Self::ID_GUESS,
            ..Self::default()
        }
    }
}

/// Memory plan: the records held per catchment, facility and zone, a few
/// geometries (selected, candidate, difference, their GEOS and edge copies)
/// and one population row; scratch for the geometry spool.
fn optimization_plan(
    grid: &GridSpec,
    settings: &ExecutionSettings,
    sizes: &Sizes,
) -> Result<ResourcePlan> {
    // Per catchment, at once: its record and identifier (as a JSON value,
    // a matching key and in its duplicate group: 3 copies), spool slot and
    // envelope, duplicate key and index, selection entry, candidate, row
    // of the table: 512 bytes with map and vector overheads.
    let catchments = 512 * sizes.catchments + 3 * sizes.catchment_id_bytes;
    // Per facility: its key (twice: point list and zone map) and point.
    let facilities = 192 * sizes.facilities + 2 * sizes.facility_id_bytes;
    // Per zone feature: its zone index, identifier and label.
    let zones = 160 * sizes.zones + sizes.zone_text_bytes;
    // Four geometries at once, each as WKB, GEOS and ring coordinates (with
    // the coverage edges): about 4 × 8 times their WKB size.
    let geometries = 32 * sizes.largest_wkb;
    // Fractions, offsets and the population row, per column.
    let rows = 32 * grid.cols as u64;
    let records = (catchments + facilities + zones) as u64;
    let mut plan = ResourcePlan::for_tiles(
        grid.rows,
        grid.cols,
        settings,
        0,
        records + geometries + rows,
        0,
    )?;
    // The spool holds the current geometry of every catchment and is
    // compacted beyond twice that plus a slack; reduced geometries may hold
    // more vertices than the input (new intersections): twice the input.
    plan.scratch_bytes = 4 * sizes.total_wkb + SPOOL_SLACK + sizes.largest_wkb;
    if let Some(free) = plan.free_disk_bytes
        && free < plan.scratch_bytes
    {
        return Err(Error::Validation {
            issues: vec![format!(
                "scratch directory {} has {free} bytes free, the catchments need {}; use \
                 --tmp-dir",
                settings.tmp_dir.display(),
                plan.scratch_bytes
            )],
        });
    }
    Ok(plan)
}

/// Validate the job against its inputs and read identifiers and zones.
pub fn prepare(
    job: OptimizationJob,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<PreparedOptimization> {
    let cancelled = monitor.cancelled();
    monitor.stage("validate");
    let started = Instant::now();
    let mut issues = job.check();
    issues.extend(check_files(
        &job.output_dir,
        job.inputs()
            .into_iter()
            .map(|(role, path)| (role.to_owned(), path)),
    ));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let (_, grid) = GridSpec::open_raster(&job.population, "population")?;
    open_aligned_raster(&job.population, &grid, "population")?;
    let catchment_layer = open_layer(&job.catchments, &grid, "catchments")?;
    // Plan from feature counts first: records are held only once it fits.
    let count = |vector: &VectorLayer| -> Result<usize> {
        let dataset = vector.dataset()?;
        Ok(vector.layer(&dataset)?.feature_count() as usize)
    };
    let mut sizes = Sizes::guess(count(&catchment_layer)?, 0, 0);
    if let Some(quota) = &job.zone_quota {
        let guess = Sizes::guess(
            0,
            count(&open_layer(&quota.facilities, &grid, "facilities")?)?,
            count(&open_zones(&quota.zones, &grid)?)?,
        );
        sizes.facilities = guess.facilities;
        sizes.facility_id_bytes = guess.facility_id_bytes;
        sizes.zones = guess.zones;
        sizes.zone_text_bytes = guess.zone_text_bytes;
    }
    optimization_plan(&grid, settings, &sizes)?.configure_gdal()?;
    let scan = scan_catchments(&catchment_layer, &job.catchments, cancelled)?;
    sizes.catchments = scan.ids.len();
    sizes.catchment_id_bytes = scan.id_bytes;
    sizes.largest_wkb = scan.largest_wkb;
    sizes.total_wkb = scan.total_wkb;
    let mut warnings = Vec::new();
    let mut catchments: Vec<CatchmentRecord> = scan
        .ids
        .iter()
        .map(|id| CatchmentRecord {
            id: id.clone(),
            zone: None,
        })
        .collect();
    let mut zones = Vec::new();
    if let Some(quota) = &job.zone_quota {
        optimization_plan(&grid, settings, &sizes)?;
        let zone_vector = open_zones(&quota.zones, &grid)?;
        let (read, feature_zones) = read_zones(&zone_vector, &quota.zones, cancelled)?;
        zones = read;
        sizes.zone_text_bytes = zones
            .iter()
            .map(|z| z.id.to_string().len() + z.label.as_ref().map_or(0, String::len))
            .sum();
        let FacilityZones {
            zone_of,
            key_bytes,
            largest_zone_wkb,
        } = facility_zones(quota, &grid, &feature_zones, cancelled)?;
        sizes.largest_wkb = sizes.largest_wkb.max(largest_zone_wkb);
        sizes.facilities = zone_of.len();
        sizes.facility_id_bytes = key_bytes;
        let mut missing = Vec::new();
        for catchment in &mut catchments {
            match zone_of.get(&id_key(&catchment.id)) {
                Some(zone) => catchment.zone = *zone,
                None => missing.push(id_key(&catchment.id)),
            }
        }
        if !missing.is_empty() {
            let preview: Vec<&str> = missing.iter().take(5).map(String::as_str).collect();
            return Err(Error::Validation {
                issues: vec![format!(
                    "{} catchments have no facility with the same {:?} in \
                     zone_quota.facilities ({:?}): {}{}",
                    missing.len(),
                    job.catchments.id_field,
                    quota.facilities.id_field,
                    preview.join(", "),
                    if missing.len() > 5 { ", ..." } else { "" }
                )],
            });
        }
        let outside = catchments.iter().filter(|c| c.zone.is_none()).count();
        if outside > 0 {
            warnings.push(Warning {
                code: "facilities_outside_zones",
                message: format!(
                    "{outside} catchments have their facility in no zone: they count for no \
                     zone minimum"
                ),
            });
        }
        let with_catchments = required_zones(&catchments, zones.len());
        let required = with_catchments.iter().filter(|&&r| r).count();
        if required
            .checked_mul(quota.min_per_zone)
            .is_none_or(|needed| needed > job.max_facilities)
        {
            return Err(Error::Validation {
                issues: vec![format!(
                    "{required} zones hold catchments: {required} x min_per_zone {} exceeds \
                     max_facilities {}",
                    quota.min_per_zone, job.max_facilities
                )],
            });
        }
    }
    let plan = optimization_plan(&grid, settings, &sizes)?;
    if catchments.is_empty() {
        warnings.push(Warning {
            code: "no_catchment",
            message: "the catchment layer has no feature: the table is empty".into(),
        });
    }
    let mut durations_s = BTreeMap::new();
    durations_s.insert("validate", started.elapsed().as_secs_f64());
    monitor.stage("fingerprints");
    let started = Instant::now();
    let fingerprints = fingerprints(&job.inputs(), cancelled)?;
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());
    Ok(PreparedOptimization {
        job,
        grid,
        catchments,
        zones,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// Zones holding at least one catchment: only they get a quota (AccessMod
/// 5 lists the zones from the catchments).
fn required_zones(catchments: &[CatchmentRecord], zones: usize) -> Vec<bool> {
    let mut required = vec![false; zones];
    for zone in catchments.iter().filter_map(|c| c.zone) {
        required[zone] = true;
    }
    required
}

/// A catchment in the selection: its current (reduced) geometry in the
/// spool, and its populations.
#[derive(Debug, Clone, Copy)]
struct Entry {
    catchment: usize,
    slot: SpoolSlot,
    envelope: [f64; 4],
    candidate: Candidate,
}

fn envelope_of(e: Envelope) -> [f64; 4] {
    [e.MinX, e.MinY, e.MaxX, e.MaxY]
}

fn boxes_meet(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// A row of the result table.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Selected {
    /// Rank, from 1.
    pub rank: usize,
    /// Catchment (index in layer order).
    pub catchment: usize,
    /// Population of the whole catchment.
    pub population_catchment: f64,
    /// Population it adds to the coverage.
    pub population_covered: f64,
}

/// Why the selection stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// `max_facilities` selected.
    MaxFacilities,
    /// No catchment left.
    NoCatchmentLeft,
}

/// Result counters.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OptimizationSummary {
    /// Catchments read.
    pub catchments: usize,
    /// Catchments sharing the geometry of an earlier one (ranked with it).
    pub duplicate_catchments: usize,
    /// Facilities selected.
    pub facilities_selected: usize,
    /// Population covered by the facilities selected.
    pub population_covered: f64,
    /// Catchment differences computed.
    pub differences: u64,
    /// Why the selection stopped.
    pub stop_reason: StopReason,
}

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct OptimizationRecord {
    /// Job with resolved paths and defaults.
    pub job: OptimizationJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// Population grid.
    pub grid: GridReport,
    /// Totals.
    pub result: OptimizationSummary,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Execution details of the optimization.
#[derive(Debug, Clone, Serialize)]
pub struct OptimizationDetails {
    /// Largest size of the geometry spool, bytes.
    pub spool_peak_bytes: u64,
}

/// `run.json` of the optimization.
pub type OptimizationReport = Report<OptimizationRecord, OptimizationDetails>;

/// Deviations from AccessMod 5 that apply to every run.
pub fn deviations(quota: bool) -> Vec<Deviation> {
    let mut deviations = vec![Deviation {
        code: "optimization_duplicates",
        message: "Catchments sharing a geometry are ranked once, under the identifiers of \
                  all of them; shared means the same coordinates (AccessMod 5 names the \
                  groups with st_equals, a topological equality, and keeps one per \
                  identical coordinates).",
    }];
    if quota {
        deviations.push(Deviation {
            code: "optimization_zone_quota",
            message: "While a zone lacks facilities, only candidates of such zones compete \
                      (AccessMod 5 takes the best value among them, then any candidate with \
                      that value); when they have no candidate left, every candidate \
                      competes (AccessMod 5 fails). Zones are told apart by identifier \
                      (AccessMod 5: by label). A facility in no zone counts for no zone \
                      and keeps its row (AccessMod 5 drops it from the table).",
        });
    }
    deviations
}

/// Rank the facilities and write every output atomically.
pub fn run(
    mut prepared: PreparedOptimization,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<(PathBuf, OptimizationReport)> {
    let cancelled = monitor.cancelled();
    let job = &prepared.job;
    let grid = &prepared.grid;
    let staging = Staging::create(&job.output_dir)?;

    // Catchments to the spool, in the grid CRS; duplicates share a slot.
    monitor.stage("read");
    let started = Instant::now();
    let mut spool = GeometrySpool::new(&settings.tmp_dir, "optimization")?;
    let vector = open_layer(&job.catchments, grid, "catchments")?;
    let mut slots: Vec<(SpoolSlot, [f64; 4])> = Vec::with_capacity(prepared.catchments.len());
    {
        let dataset = vector.dataset()?;
        let mut layer = vector.layer(&dataset)?;
        for (k, feature) in layer.features().enumerate() {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let geometry = feature.geometry().ok_or_else(|| {
                vector.feature_problem(feature.fid(), k as u64)("has no geometry")
            })?;
            let geometry = vector
                .to_grid_geometry(geometry)
                .map_err(|why| vector.feature_problem(feature.fid(), k as u64)(why))?;
            slots.push((spool.push(&geometry)?, envelope_of(geometry.envelope())));
        }
    }
    if slots.len() != prepared.catchments.len() {
        return Err(Error::invalid(format!(
            "catchments {}: the layer changed since validation",
            job.catchments.path.display()
        )));
    }
    // Same coordinates: same envelope and WKB; compare within envelopes.
    let mut first_of: Vec<usize> = (0..slots.len()).collect();
    let mut by_box: BTreeMap<[u64; 5], Vec<usize>> = BTreeMap::new();
    for (i, (slot, envelope)) in slots.iter().enumerate() {
        let key = [
            envelope[0].to_bits(),
            envelope[1].to_bits(),
            envelope[2].to_bits(),
            envelope[3].to_bits(),
            slot.len(),
        ];
        by_box.entry(key).or_default().push(i);
    }
    for group in by_box.values().filter(|g| g.len() > 1) {
        for (n, &i) in group.iter().enumerate() {
            if first_of[i] != i {
                continue;
            }
            let wkb = spool.wkb(slots[i].0)?;
            for &j in &group[n + 1..] {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                if first_of[j] == j && spool.wkb(slots[j].0)? == wkb {
                    first_of[j] = i;
                }
            }
        }
    }
    let duplicates = (0..slots.len()).filter(|&i| first_of[i] != i).count();
    prepared
        .durations_s
        .insert("read", started.elapsed().as_secs_f64());

    // Population of each catchment.
    monitor.stage("population");
    let started = Instant::now();
    let (population_ds, alignment) = open_aligned_raster(&job.population, grid, "population")?;
    let population = AlignedBand::new(&population_ds, &job.population, alignment, grid)?;
    let mut alive = Vec::new();
    for (i, &(slot, envelope)) in slots.iter().enumerate() {
        if first_of[i] != i {
            continue;
        }
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let people = covered_sum(&spool.get(slot)?, grid, &population, cancelled)?;
        alive.push(Entry {
            catchment: i,
            slot,
            envelope,
            candidate: Candidate {
                covered: people,
                population: people,
                zone: prepared.catchments[i].zone,
            },
        });
    }
    prepared
        .durations_s
        .insert("population", started.elapsed().as_secs_f64());

    // Greedy selection.
    monitor.stage("selection");
    let started = Instant::now();
    let mut quota = job.zone_quota.as_ref().map(|q| {
        ZoneQuota::new(
            &required_zones(&prepared.catchments, prepared.zones.len()),
            q.min_per_zone,
        )
    });
    let mut selected: Vec<Selected> = Vec::new();
    // Catchments entirely inside selected ones, appended at the end when
    // fewer than `max_facilities` were selected (as AccessMod 5).
    let mut contained: Vec<Entry> = Vec::new();
    let mut differences = 0u64;
    let mut spool_peak = spool.bytes();
    let mut candidates: Vec<Candidate> = Vec::new();
    while selected.len() < job.max_facilities && !alive.is_empty() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        monitor.advance(
            "selection",
            selected.len() as u64,
            Some(job.max_facilities as u64),
        );
        monitor.advance(
            "selection",
            selected.len() as u64,
            Some(job.max_facilities as u64),
        );
        candidates.clear();
        candidates.extend(alive.iter().map(|e| e.candidate));
        let pick = select(&candidates, quota.as_ref()).expect("candidates are left");
        let chosen = alive.remove(pick);
        if let Some(quota) = &mut quota {
            quota.record(chosen.candidate.zone);
        }
        selected.push(Selected {
            rank: selected.len() + 1,
            catchment: chosen.catchment,
            population_catchment: chosen.candidate.population,
            population_covered: chosen.candidate.covered,
        });
        if alive.is_empty() {
            break;
        }
        let taken = spool.get(chosen.slot)?;
        let mut any = false;
        for entry in &mut alive {
            if !boxes_meet(&entry.envelope, &chosen.envelope) {
                continue;
            }
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let geometry = spool.get(entry.slot)?;
            if !geometry.intersects(&taken) {
                continue;
            }
            any = true;
            differences += 1;
            let rest = geometry.difference(&taken).ok_or_else(|| {
                Error::invalid(format!(
                    "catchments {}: GEOS failed to subtract catchment {} from catchment {}",
                    job.catchments.path.display(),
                    id_key(&prepared.catchments[chosen.catchment].id),
                    id_key(&prepared.catchments[entry.catchment].id)
                ))
            })?;
            if rest.is_empty() {
                entry.candidate.covered = 0.0;
                contained.push(*entry);
                continue;
            }
            entry.candidate.covered = covered_sum(&rest, grid, &population, cancelled)?;
            entry.slot = spool.push(&rest)?;
            entry.envelope = envelope_of(rest.envelope());
        }
        // AccessMod 5 drops candidates covering no one only after a step
        // that reduced some catchment.
        if any {
            alive.retain(|e| e.candidate.covered > 0.0);
        }
        // Keep only current geometries once the spool holds twice them.
        spool_peak = spool_peak.max(spool.bytes());
        let live: u64 = alive.iter().map(|e| e.slot.len()).sum();
        if spool.bytes() > 2 * live + SPOOL_SLACK {
            spool.compact(alive.iter_mut().map(|e| &mut e.slot))?;
        }
    }
    let stop_reason = if selected.len() >= job.max_facilities {
        StopReason::MaxFacilities
    } else {
        StopReason::NoCatchmentLeft
    };
    for entry in contained {
        if selected.len() >= job.max_facilities {
            break;
        }
        selected.push(Selected {
            rank: selected.len() + 1,
            catchment: entry.catchment,
            population_catchment: entry.candidate.population,
            population_covered: 0.0,
        });
    }
    prepared
        .durations_s
        .insert("selection", started.elapsed().as_secs_f64());

    // Table.
    let mut group_ids: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (i, &first) in first_of.iter().enumerate() {
        group_ids
            .entry(first)
            .or_default()
            .push(id_key(&prepared.catchments[i].id));
    }
    let with_zones = job.zone_quota.is_some();
    let mut header =
        String::from("rank,facility_id,facility_ids,population_catchment,population_covered");
    if with_zones {
        header.push_str(",zone_id,zone_label");
    }
    header.push_str(",population_covered_cumulative");
    let mut csv = CsvWriter::create(&staging.path().join(OPTIMIZATION_FILE), &header)?;
    let mut cumulative = 0.0;
    for row in &selected {
        cumulative += row.population_covered;
        let catchment = &prepared.catchments[row.catchment];
        let ids = group_ids[&row.catchment].join(" // ");
        let mut fields = vec![
            row.rank.to_string(),
            csv_field(&catchment.id),
            csv_field(&serde_json::Value::String(ids)),
            format!("{}", row.population_catchment),
            format!("{}", row.population_covered),
        ];
        if with_zones {
            let zone = catchment.zone.map(|z| &prepared.zones[z]);
            fields.push(zone.map(|z| csv_field(&z.id)).unwrap_or_default());
            fields.push(
                zone.and_then(|z| z.label.clone())
                    .map(|l| csv_field(&serde_json::Value::String(l)))
                    .unwrap_or_default(),
            );
        }
        fields.push(format!("{cumulative}"));
        csv.record(&fields)?;
    }
    csv.finish()?;

    let mut warnings = prepared.warnings.clone();
    if duplicates > 0 {
        warnings.push(Warning {
            code: "duplicate_catchments",
            message: format!(
                "{duplicates} catchments share the geometry of an earlier one: each group is \
                 ranked once (facility_ids lists them)"
            ),
        });
    }
    if selected.len() < job.max_facilities {
        warnings.push(Warning {
            code: "fewer_facilities",
            message: format!(
                "{} facilities selected of the {} requested: no catchment left",
                selected.len(),
                job.max_facilities
            ),
        });
    }
    let summary = OptimizationSummary {
        catchments: prepared.catchments.len(),
        duplicate_catchments: duplicates,
        facilities_selected: selected.len(),
        population_covered: cumulative,
        differences,
        stop_reason,
    };
    let details = OptimizationDetails {
        spool_peak_bytes: spool_peak.max(spool.bytes()),
    };
    let PreparedOptimization {
        job,
        grid,
        plan,
        fingerprints,
        durations_s,
        ..
    } = prepared;
    let report = Report::new(
        OptimizationRecord {
            deviations: deviations(job.zone_quota.is_some()),
            job,
            inputs: fingerprints,
            grid: GridReport::of(&grid),
            result: summary,
            warnings,
        },
        Execution::now(settings, plan, durations_s, details),
    );
    let output_dir = output::finish(staging, &report.run.job, &report, cancelled)?;
    Ok((output_dir, report))
}

/// Optimization.
#[derive(Debug)]
pub struct Optimization;

impl Analysis for Optimization {
    const NAME: &'static str = "optimization";
    const OUTPUTS: &'static [Output] = &[Output {
        file: OPTIMIZATION_FILE,
        class: Some(DataClass::OptimizationResult),
    }];
    type Job = OptimizationJob;
    type Prepared = PreparedOptimization;
    type Report = OptimizationReport;

    fn prepare(
        job: OptimizationJob,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<PreparedOptimization> {
        prepare(job, settings, monitor)
    }

    fn run(
        prepared: PreparedOptimization,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<(PathBuf, OptimizationReport)> {
        run(prepared, settings, monitor)
    }

    fn warnings(prepared: &PreparedOptimization) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &PreparedOptimization) -> String {
        format!(
            "optimization: {} catchments, up to {} facilities{} on a {} x {} grid",
            prepared.catchments.len(),
            prepared.job.max_facilities,
            match &prepared.job.zone_quota {
                Some(q) => format!(", at least {} per zone", q.min_per_zone),
                None => String::new(),
            },
            prepared.grid.cols,
            prepared.grid.rows
        )
    }

    fn validation(prepared: &PreparedOptimization) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "catchments": prepared.catchments,
            "zones": prepared.zones,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &OptimizationReport) -> String {
        let r = &report.run.result;
        format!(
            "{} facilities selected of {} catchments; population covered {}",
            r.facilities_selected, r.catchments, r.population_covered
        )
    }
}
