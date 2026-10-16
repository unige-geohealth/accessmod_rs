//! Zonal statistics (schema version 1): population covered within
//! travel-time limits, per zone, as AccessMod 5 does
//! (`tools/R/amAnalysisZonal.R`; rule in `accessmod_core::zonal`).
//!
//! Zones are burnt by cell centre; features sharing an identifier form one
//! zone, and a cell in several zones belongs to the first in layer order.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use accessmod_core::zonal::ZoneCoverage;
use gdal::vector::LayerAccess;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Cancelled};
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::{JobFile, absolutize, check_files, check_schema_version};
use crate::output::{self, BLOCK, CsvWriter, Staging, csv_field};
use crate::raster::{AlignedBand, for_each_block, open_aligned_raster};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::vector::{VectorLayer, burn_zones, compare_values, json_attribute};

/// Zonal statistics file name.
pub const ZONAL_FILE: &str = "zonal_statistics.csv";

/// Zones a job may hold (zone indices are 32-bit, 0 is "no zone").
pub(crate) const MAX_ZONES: usize = u32::MAX as usize - 1;

/// Zonal statistics job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZonalStatisticsJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Digital elevation model; defines the grid. Its values are not used.
    pub dem: PathBuf,
    /// Travel time, seconds (`travel_time_seconds.tif` of a travel-time run),
    /// on the DEM lattice; NoData cells are not reached.
    pub travel_time: PathBuf,
    /// Population raster (people per cell), on the DEM lattice.
    pub population: PathBuf,
    /// Zones.
    pub zones: ZoneLayer,
    /// Travel-time limits, seconds (> 0); duplicates are removed and the
    /// limits sorted.
    pub time_limits_s: Vec<f64>,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

/// Zone polygons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZoneLayer {
    /// Vector dataset with polygons.
    pub path: PathBuf,
    /// Layer name; the first layer when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Field holding the zone identifier; features sharing it form one zone.
    pub id_field: String,
    /// Field holding the zone label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_field: Option<String>,
    /// Optional OGR SQL attribute filter.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

impl JobFile for ZonalStatisticsJob {
    fn resolve_paths(mut self, base: &Path) -> Self {
        for path in [
            &mut self.dem,
            &mut self.travel_time,
            &mut self.population,
            &mut self.zones.path,
            &mut self.output_dir,
        ] {
            absolutize(path, base);
        }
        self
    }
}

impl ZonalStatisticsJob {
    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues: Vec<String> = check_schema_version(self.schema_version)
            .into_iter()
            .collect();
        if self.zones.id_field.trim().is_empty() {
            issues.push("zones.id_field is empty".into());
        }
        if self.time_limits_s.is_empty() {
            issues.push("time_limits_s is empty".into());
        }
        for limit in &self.time_limits_s {
            if !(limit.is_finite() && *limit > 0.0) {
                issues.push(format!("time_limits_s: {limit} must be finite and > 0"));
            }
        }
        issues
    }

    /// Limits, sorted, without duplicates (call after [`Self::check`]).
    pub fn limits(&self) -> Vec<f64> {
        let mut limits = self.time_limits_s.clone();
        limits.sort_by(f64::total_cmp);
        limits.dedup();
        limits
    }
}

/// A zone: identifier and label.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Zone {
    /// Identifier.
    pub id: serde_json::Value,
    /// Label (first non-empty among the zone's features).
    pub label: Option<String>,
}

/// Inputs validated, ready to compute.
#[derive(Debug)]
pub struct PreparedZonal {
    /// Job with resolved paths.
    pub job: ZonalStatisticsJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Zones in order of first appearance (zone index = position + 1).
    pub zones: Vec<Zone>,
    /// Zone index of each feature, in layer order.
    pub feature_zones: Vec<u32>,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Input files.
    pub fingerprints: Vec<Fingerprint>,
    /// Seconds spent per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
}

/// Validate the job against its inputs and read the zones.
pub fn prepare(
    job: ZonalStatisticsJob,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedZonal> {
    let started = Instant::now();
    let mut issues = job.check();
    issues.extend(check_files(
        &job.output_dir,
        [
            ("dem".to_owned(), job.dem.as_path()),
            ("travel_time".to_owned(), job.travel_time.as_path()),
            ("population".to_owned(), job.population.as_path()),
            ("zones.path".to_owned(), job.zones.path.as_path()),
        ],
    ));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let (_, grid) = GridSpec::open_dem(&job.dem)?;
    zonal_plan(&grid, settings, 0, 0)?.configure_gdal()?;
    for (path, role) in [
        (&job.travel_time, "travel time"),
        (&job.population, "population"),
    ] {
        match open_aligned_raster(path, &grid, role) {
            Ok(_) => {}
            Err(Error::Validation { issues: more }) => issues.extend(more),
            Err(other) => return Err(other),
        }
    }
    let vector = open_zones(&job.zones, &grid)?;
    let count = {
        let dataset = vector.dataset()?;
        vector.layer(&dataset)?.feature_count() as usize
    };
    if count > MAX_ZONES {
        issues.push(format!("zones: more than {MAX_ZONES} features"));
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let limits = job.limits().len();
    let plan = zonal_plan(&grid, settings, count, limits)?;
    let (zones, feature_zones) = read_zones(&vector, &job.zones, cancelled)?;
    let mut warnings = Vec::new();
    if zones.is_empty() {
        warnings.push(Warning {
            code: "no_zone",
            message: "the zone layer has no feature: the table is empty".into(),
        });
    }
    let mut durations_s = BTreeMap::new();
    durations_s.insert("validate", started.elapsed().as_secs_f64());
    let started = Instant::now();
    let fingerprints = fingerprints(
        &[
            ("dem", job.dem.as_path()),
            ("travel_time", job.travel_time.as_path()),
            ("population", job.population.as_path()),
            ("zones", job.zones.path.as_path()),
        ],
        cancelled,
    )?;
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());
    Ok(PreparedZonal {
        job,
        grid,
        zones,
        feature_zones,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// Memory plan: the zone index grid, block buffers, and per zone its
/// coverage and identifier (`features` zones at most).
fn zonal_plan(
    grid: &GridSpec,
    settings: &ExecutionSettings,
    features: usize,
    limits: usize,
) -> Result<ResourcePlan> {
    let geometry = TileGeometry::new(grid.rows, grid.cols, accessmod_core::store::DEFAULT_TILE)?;
    let per_tile = TiledGrid::<u32>::tile_bytes(geometry) as u64;
    // Identifier and label are held per zone; count 128 bytes for them.
    let per_zone = std::mem::size_of::<ZoneCoverage>() + 8 * limits + 128 + 4;
    // Per block: two bands read as f64 then held as Option<f64>, zones.
    let buffer_bytes = (BLOCK * BLOCK * (2 * (8 + 16) + 4) + features * per_zone) as u64;
    ResourcePlan::for_tiles(grid.rows, grid.cols, settings, per_tile, buffer_bytes, 0)
}

pub(crate) fn open_zones(source: &ZoneLayer, grid: &GridSpec) -> Result<VectorLayer> {
    VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        "zones",
    )
}

/// Zones by identifier, in order of first appearance, and the zone of each
/// feature.
pub(crate) fn read_zones(
    vector: &VectorLayer,
    source: &ZoneLayer,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<Zone>, Vec<u32>)> {
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let id_index = vector.field_index(&layer, &source.id_field)?;
    let label_index = match &source.label_field {
        Some(field) => Some(vector.field_index(&layer, field)?),
        None => None,
    };
    let mut zones: Vec<Zone> = Vec::new();
    let mut index_of: BTreeMap<String, u32> = BTreeMap::new();
    let mut feature_zones = Vec::new();
    let mut issues = Vec::new();
    for (k, feature) in layer.features().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let Some(id) = json_attribute(feature.field(id_index).map_err(Error::gdal(&source.path))?)
        else {
            let which = feature
                .fid()
                .map_or_else(|| format!("#{k}"), |fid| format!("FID {fid}"));
            issues.push(format!(
                "zones feature {which}: empty {:?}",
                source.id_field
            ));
            continue;
        };
        let label = match label_index {
            Some(i) => feature
                .field_as_string(i)
                .map_err(Error::gdal(&source.path))?
                .filter(|l| !l.trim().is_empty()),
            None => None,
        };
        let zone = *index_of.entry(id.to_string()).or_insert_with(|| {
            zones.push(Zone {
                id: id.clone(),
                label: None,
            });
            zones.len() as u32
        });
        let entry = &mut zones[zone as usize - 1];
        if entry.label.is_none() {
            entry.label = label;
        }
        feature_zones.push(zone);
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    Ok((zones, feature_zones))
}

/// Totals within one limit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LimitSummary {
    /// Limit, seconds.
    pub time_limit_s: f64,
    /// Population covered within the limit, all zones.
    pub population_covered: f64,
}

/// Result counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ZonalSummary {
    /// Zones.
    pub zones: usize,
    /// Population inside the zones.
    pub population: f64,
    /// Population outside every zone.
    pub population_outside_zones: f64,
    /// Totals per limit.
    pub limits: Vec<LimitSummary>,
    /// Burns of a cell already in an earlier zone (kept there): one per
    /// extra zone covering the cell.
    pub overlapping_cells: u64,
}

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct ZonalRecord {
    /// Job with resolved paths and defaults.
    pub job: ZonalStatisticsJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// DEM grid.
    pub grid: GridReport,
    /// Totals.
    pub result: ZonalSummary,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Execution details of zonal statistics.
#[derive(Debug, Clone, Serialize)]
pub struct ZonalStore {
    /// Cache counters of the zone grid.
    pub store: StoreStats,
}

/// `run.json` of zonal statistics.
pub type ZonalReport = Report<ZonalRecord, ZonalStore>;

/// Deviations from AccessMod 5 that apply to every run.
pub fn deviations() -> Vec<Deviation> {
    vec![
        Deviation {
            code: "zonal_limit_seconds",
            message: "Cells are covered when 0 <= travel time <= limit, in seconds. AccessMod \
                      5 compares its travel time, rounded up to whole minutes, with t < limit: \
                      a limit of L minutes there equals (L - 1) x 60 s here on the same \
                      minutes.",
        },
        Deviation {
            code: "overlapping_zones",
            message: "A cell in several zones belongs to the first zone in layer order. \
                      AccessMod 5 rasterises zones with v.to.rast, where the last one wins.",
        },
    ]
}

/// Compute the statistics and write every output atomically.
pub fn run(
    mut prepared: PreparedZonal,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<(PathBuf, ZonalReport)> {
    let job = &prepared.job;
    let grid = &prepared.grid;
    let staging = Staging::create(&job.output_dir)?;
    let geometry = TileGeometry::new(grid.rows, grid.cols, prepared.plan.tile)?;
    let mut zone_grid = TiledGrid::new(
        geometry,
        0u32,
        prepared.plan.resident_tiles,
        &settings.tmp_dir,
        "zones",
    )?;

    let started = Instant::now();
    let vector = open_zones(&job.zones, grid)?;
    let mut feature = 0usize;
    let (_, overlaps) = burn_zones(&vector, grid, &mut zone_grid, cancelled, |_| {
        feature += 1;
        Ok(prepared.feature_zones.get(feature - 1).copied())
    })?;
    prepared
        .durations_s
        .insert("zones", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let (time_ds, time_alignment) = open_aligned_raster(&job.travel_time, grid, "travel time")?;
    let (population_ds, population_alignment) =
        open_aligned_raster(&job.population, grid, "population")?;
    let travel_time = AlignedBand::new(&time_ds, &job.travel_time, time_alignment, grid)?;
    let population = AlignedBand::new(&population_ds, &job.population, population_alignment, grid)?;
    let limits = job.limits();
    let mut coverage = vec![ZoneCoverage::new(limits.len()); prepared.zones.len() + 1];
    let mut outside = 0.0;
    // Band 0: people per cell; band 1: travel time (`None`: not reached).
    for_each_block(
        grid,
        (BLOCK, BLOCK),
        &[&population, &travel_time],
        Some(&mut zone_grid),
        cancelled,
        |block| {
            for i in 0..block.len() {
                let Some(people) = block.value(0, i) else {
                    continue;
                };
                match block.zone(i) {
                    0 => outside += people,
                    zone => coverage[zone as usize].add(people, block.value(1, i), &limits),
                }
            }
            Ok(())
        },
    )?;

    let mut summary = ZonalSummary {
        zones: prepared.zones.len(),
        population_outside_zones: outside,
        overlapping_cells: overlaps,
        ..ZonalSummary::default()
    };
    let mut csv = CsvWriter::create(
        &staging.path().join(ZONAL_FILE),
        "zone_id,label,time_limit_s,population,population_covered,coverage_percent",
    )?;
    let zones = &coverage[1..];
    summary.population = zones.iter().map(|z| z.population).sum();
    for (k, &limit) in limits.iter().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        summary.limits.push(LimitSummary {
            time_limit_s: limit,
            population_covered: zones.iter().map(|z| z.covered[k]).sum(),
        });
        // As AccessMod 5: by coverage, ascending; ties by identifier (numbers
        // numerically), as its merge sorts the zones.
        let mut order: Vec<usize> = (0..zones.len()).collect();
        order.sort_by(|&a, &b| {
            zones[a]
                .percent(k)
                .total_cmp(&zones[b].percent(k))
                .then_with(|| compare_values(&prepared.zones[a].id, &prepared.zones[b].id))
        });
        for z in order {
            let zone = &prepared.zones[z];
            let label = zone
                .label
                .as_ref()
                .map(|l| csv_field(&serde_json::Value::String(l.clone())))
                .unwrap_or_default();
            csv.record(&[
                csv_field(&zone.id),
                label,
                format!("{limit}"),
                format!("{}", zones[z].population),
                format!("{}", zones[z].covered[k]),
                format!("{}", zones[z].percent(k)),
            ])?;
        }
    }
    csv.finish()?;
    prepared
        .durations_s
        .insert("statistics", started.elapsed().as_secs_f64());

    let mut warnings = prepared.warnings.clone();
    if overlaps > 0 {
        warnings.push(Warning {
            code: "overlapping_zones",
            message: format!(
                "{overlaps} cells lie in several zones; each belongs to the first zone in \
                 layer order"
            ),
        });
    }
    let store = zone_grid.stats();
    let PreparedZonal {
        job,
        grid,
        plan,
        fingerprints,
        durations_s,
        ..
    } = prepared;
    let report = Report::new(
        ZonalRecord {
            job,
            inputs: fingerprints,
            grid: GridReport::of(&grid),
            result: summary,
            warnings,
            deviations: deviations(),
        },
        Execution::now(settings, plan, durations_s, ZonalStore { store }),
    );
    let output_dir = output::finish(staging, &report.run.job, &report, cancelled)?;
    Ok((output_dir, report))
}

/// Zonal statistics.
#[derive(Debug)]
pub struct ZonalStatistics;

impl Analysis for ZonalStatistics {
    type Job = ZonalStatisticsJob;
    type Prepared = PreparedZonal;
    type Report = ZonalReport;

    fn prepare(
        job: ZonalStatisticsJob,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<PreparedZonal> {
        prepare(job, settings, cancelled)
    }

    fn run(
        prepared: PreparedZonal,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<(PathBuf, ZonalReport)> {
        run(prepared, settings, cancelled)
    }

    fn warnings(prepared: &PreparedZonal) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &PreparedZonal) -> String {
        format!(
            "zonal statistics: {} zones, {} limits on a {} x {} grid",
            prepared.zones.len(),
            prepared.job.limits().len(),
            prepared.grid.cols,
            prepared.grid.rows
        )
    }

    fn validation(prepared: &PreparedZonal) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "zones": prepared.zones,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &ZonalReport) -> String {
        let r = &report.run.result;
        let covered: Vec<String> = r
            .limits
            .iter()
            .map(|l| format!("{} s: {}", l.time_limit_s, l.population_covered))
            .collect();
        format!(
            "population {} in {} zones; covered {}",
            r.population,
            r.zones,
            covered.join(", ")
        )
    }
}
