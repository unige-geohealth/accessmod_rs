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
//!
//! Optional outputs: the population on barriers (people the analysis cannot
//! reach) and, with zone polygons, the population covered per zone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::capacity::{Catchment, catchment};
use accessmod_core::cost::{CellMobility, Direction, Method, Neighbourhood};
use accessmod_core::mobility::MobilityModel;
use accessmod_core::solver::{Facility, InputCell, SolverConfig};
use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use gdal::vector::{FieldValue, LayerAccess, OGRFieldType, OGRwkbGeometryType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Monitor, Output};
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
use crate::travel_time::{Landscape, landscape_warnings, load_landscape, screen_facilities};
use crate::vector::{FeatureWriter, VectorLayer, burn_zones, compare_values, number_attribute};
use crate::zonal::{MAX_ZONES, Zone, ZoneLayer, open_zones, read_zones};

/// Capacity table file name.
pub const CAPACITY_FILE: &str = "capacity.csv";
/// Residual population file name.
pub const RESIDUAL_FILE: &str = "population_residual.tif";
/// Catchment polygons file name.
pub const CATCHMENTS_FILE: &str = "catchments.fgb";
/// Population on barriers file name.
pub const BARRIERS_FILE: &str = "population_on_barriers.tif";
/// Coverage per zone file name.
pub const ZONES_FILE: &str = "zonal_coverage.csv";

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
    #[schemars(extend("x-accessmod-class" = ["rDem"]))]
    pub dem: PathBuf,
    /// Land-cover raster with integer classes, on the DEM lattice.
    #[schemars(extend("x-accessmod-class" = ["rLandCoverMerged"]))]
    pub land_cover: PathBuf,
    /// Population raster (people per cell), on the DEM lattice.
    #[schemars(extend("x-accessmod-class" = ["rPopulation", "rPopulationResidual"]))]
    pub population: PathBuf,
    /// Facility point layer.
    #[schemars(extend("x-accessmod-class" = ["vFacility", "vFacilityNew"]))]
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
    /// Write the population on barriers (`population_on_barriers.tif`).
    #[serde(default)]
    pub population_on_barriers: bool,
    /// Zone polygons: write the population covered per zone
    /// (`zonal_coverage.csv`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-accessmod-class" = ["vZone"]))]
    pub zones: Option<ZoneLayer>,
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
    fn for_each_path(&mut self, f: &mut dyn FnMut(&mut PathBuf)) {
        for path in [
            &mut self.dem,
            &mut self.land_cover,
            &mut self.population,
            &mut self.facilities.path,
            &mut self.output_dir,
        ] {
            f(path);
        }
        if let Some(zones) = &mut self.zones {
            f(&mut zones.path);
        }
    }
}

impl CoverageJob {
    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues = check_common(self.schema_version, &self.scenario, Some(self.limit_s));
        if self.facilities.id_field.trim().is_empty() {
            issues.push("facilities.id_field is empty".into());
        }
        if self
            .zones
            .as_ref()
            .is_some_and(|z| z.id_field.trim().is_empty())
        {
            issues.push("zones.id_field is empty".into());
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
    /// Value the order sorts: the order field, or the population reached
    /// within the order limit.
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
    /// Zones in order of first appearance (zone index = position + 1).
    pub zones: Vec<Zone>,
    /// Zone index of each zone feature, in layer order.
    pub feature_zones: Vec<u32>,
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
    monitor: Monitor<'_>,
) -> Result<PreparedCoverage> {
    let cancelled = monitor.cancelled();
    monitor.stage("load_inputs");
    let started = Instant::now();
    let mut issues = job.check();
    let mut files = vec![
        ("dem".to_owned(), job.dem.as_path()),
        ("land_cover".to_owned(), job.land_cover.as_path()),
        ("population".to_owned(), job.population.as_path()),
        ("facilities.path".to_owned(), job.facilities.path.as_path()),
    ];
    if let Some(zones) = &job.zones {
        files.push(("zones.path".to_owned(), zones.path.as_path()));
    }
    issues.extend(check_files(&job.output_dir, files));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let (slots, scenario) = scenario_table(&job.scenario);
    // The residual population grid (f64), and the zone grid (u32) when
    // zones are set, share the tile budget.
    let geometry_probe = TileGeometry::new(1, 1, accessmod_core::store::DEFAULT_TILE)?;
    let mut residual_tile = TiledGrid::<f64>::tile_bytes(geometry_probe) as u64;
    // Per zone feature: its zone index, and at most one zone with its
    // totals, identifier and label (128 bytes counted for them, as zonal
    // statistics).
    let mut zone_bytes = 0;
    let zone_layer = match &job.zones {
        Some(source) => {
            residual_tile += TiledGrid::<u32>::tile_bytes(geometry_probe) as u64;
            let (_, dem_grid) = GridSpec::open_dem(&job.dem)?;
            let vector = open_zones(source, &dem_grid)?;
            let count = {
                let dataset = vector.dataset()?;
                vector.layer(&dataset)?.feature_count() as usize
            };
            if count > MAX_ZONES {
                return Err(Error::Validation {
                    issues: vec![format!("zones: more than {MAX_ZONES} features")],
                });
            }
            zone_bytes = (count * (4 + std::mem::size_of::<ZoneTotals>() + 128)) as u64;
            Some((vector, source))
        }
        None => None,
    };
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
        zone_bytes,
        cancelled,
    )?;
    open_aligned_raster(&job.population, &grid, "population")?;
    let loaded = load_facilities(&job.facilities, &grid, &mut inputs, cancelled)?;
    let mut warnings = Vec::new();
    screen_facilities(&loaded, job.facility_issues, "facilities", &mut warnings)?;
    warnings.extend(landscape_warnings(&summary, &job.scenario));
    let facilities = read_facilities(&job, &grid, &loaded, cancelled)?;
    let (zones, feature_zones) = match &zone_layer {
        Some((vector, source)) => read_zones(vector, source, cancelled)?,
        None => (Vec::new(), Vec::new()),
    };
    let mut durations_s = BTreeMap::new();
    durations_s.insert("load_inputs", started.elapsed().as_secs_f64());
    monitor.stage("fingerprints");
    let started = Instant::now();
    let mut files = vec![
        ("dem", job.dem.as_path()),
        ("land_cover", job.land_cover.as_path()),
        ("population", job.population.as_path()),
        ("facilities", job.facilities.path.as_path()),
    ];
    if let Some(zones) = &job.zones {
        files.push(("zones", zones.path.as_path()));
    }
    let fingerprints = fingerprints(&files, cancelled)?;
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());
    Ok(PreparedCoverage {
        job,
        grid,
        inputs,
        scenario,
        loaded,
        facilities,
        zones,
        feature_zones,
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
    /// Residual population before this facility.
    pub population_residual_before: f64,
    /// Residual population after this facility.
    pub population_residual_after: f64,
    /// Population covered so far, percent of the population off barriers.
    pub population_covered_percent: f64,
}

/// Population covered in one zone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct ZoneTotals {
    population: f64,
    on_barriers: f64,
    covered: f64,
}

/// Result counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CoverageSummary {
    /// Facilities processed.
    pub facilities: usize,
    /// Population of the raster.
    pub population_total: f64,
    /// Population on barriers, which no facility reaches.
    pub population_on_barriers: f64,
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
            code: "coverage_zonal_barriers",
            message: "Zonal coverage counts people on barriers as not covered. \
                      AccessMod 5 subtracts the residual population, which has no value \
                      on barriers, from the whole population: they count as covered.",
        },
        Deviation {
            code: "coverage_not_yet",
            message: "Not yet available: circular-buffer order.",
        },
    ]
}

/// Compute the coverage and write every output atomically.
pub fn run(
    prepared: PreparedCoverage,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<(PathBuf, CoverageReport)> {
    let cancelled = monitor.cancelled();
    let PreparedCoverage {
        job,
        grid,
        mut inputs,
        scenario,
        mut facilities,
        zones,
        feature_zones,
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

    // Residual population and population on barriers.
    monitor.stage("residual");
    let started = Instant::now();
    let (population_ds, alignment) = open_aligned_raster(&job.population, &grid, "population")?;
    let population = AlignedBand::new(&population_ds, &job.population, alignment, &grid)?;
    let barriers_tif = if job.population_on_barriers {
        Some(GeoTiff::create::<f64>(
            &staging.path().join(BARRIERS_FILE),
            &grid,
            3,
            settings.threads,
            FLOAT_NODATA,
        )?)
    } else {
        None
    };
    let PopulationTotals {
        total: population_total,
        on_barriers,
        off_barriers: total,
    } = init_residual(
        &grid,
        &population,
        &mut inputs,
        &mut residual,
        barriers_tif.as_ref(),
        cancelled,
    )?;
    if let Some(tif) = barriers_tif {
        tif.close()?;
    }
    durations_s.insert("residual", started.elapsed().as_secs_f64());

    let mut spool = CellSpool::new(&settings.tmp_dir)?;
    let state_tiles = plan.resident_tiles;

    // Processing order.
    monitor.stage("order");
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
            for (facility, value) in facilities.iter_mut().zip(&values) {
                facility.order_value = Some(*value);
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
    monitor.stage("coverage");
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
        monitor.advance("coverage", k as u64, Some(order.len() as u64));
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
        let residual_before = remaining;
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
            remaining -= remove_covered(&mut spool, &mut residual, &result, cancelled)?;
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
            population_residual_before: residual_before,
            population_residual_after: remaining,
            population_covered_percent: percent,
        });
    }
    if let Some(writer) = catchments {
        writer.close()?;
    }
    durations_s.insert("coverage", started.elapsed().as_secs_f64());

    // Coverage per zone (amAnalysisCapacity zonalCoverage): population,
    // people on barriers and population covered, the population minus the
    // residual off barriers (people <= 0 start with a residual of 0, so they
    // add their value, as in AccessMod 5).
    if let Some(source) = &job.zones {
        monitor.stage("zones");
        let started = Instant::now();
        let mut zone_grid = TiledGrid::new(
            geometry,
            0u32,
            plan.resident_tiles,
            &settings.tmp_dir,
            "zones",
        )?;
        let vector = open_zones(source, &grid)?;
        let mut feature = 0;
        burn_zones(&vector, &grid, &mut zone_grid, cancelled, |_| {
            feature += 1;
            Ok(feature_zones.get(feature - 1).copied())
        })?;
        let mut totals = vec![ZoneTotals::default(); zones.len() + 1];
        for_each_block(
            &grid,
            (BLOCK, BLOCK),
            &[&population],
            Some(&mut zone_grid),
            cancelled,
            |block| {
                let width = block.cols.len();
                for i in 0..block.len() {
                    let zone = block.zone(i);
                    let Some(people) = block.value(0, i).filter(|_| zone > 0) else {
                        continue;
                    };
                    let (row, col) = (block.rows.start + i / width, block.cols.start + i % width);
                    let entry = &mut totals[zone as usize];
                    entry.population += people;
                    let left = residual.get(row, col)?;
                    if !inputs.get(row, col)?.is_passable() {
                        entry.on_barriers += people;
                    } else if !left.is_nan() {
                        entry.covered += people - left;
                    }
                }
                Ok(())
            },
        )?;
        write_zone_table(
            &staging.path().join(ZONES_FILE),
            &zones,
            &totals[1..],
            cancelled,
        )?;
        durations_s.insert("zones", started.elapsed().as_secs_f64());
    }

    // Outputs.
    monitor.stage("write_outputs");
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
        population_total,
        population_on_barriers: on_barriers,
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
        &format!("order,facility_id,name,capacity,order_value,{CATCHMENT_COLUMNS}"),
    )?;
    for row in rows {
        let c = &row.catchment;
        let f = &row.facility;
        let mut record = vec![
            row.order.to_string(),
            csv_field(&f.id),
            f.name
                .as_ref()
                .map(|n| csv_field(&serde_json::Value::String(n.clone())))
                .unwrap_or_default(),
            table_number(Some(c.capacity)),
            table_number(f.order_value),
        ];
        record.extend(catchment_fields(
            c,
            job.limit_s,
            job.band_s,
            ResidualStep {
                before: row.population_residual_before,
                after: row.population_residual_after,
                covered_percent: row.population_covered_percent,
            },
        ));
        csv.record(&record)?;
    }
    csv.finish()
}

/// Zones by identifier (numbers numerically), as AccessMod 5 merges them.
fn write_zone_table(
    path: &Path,
    zones: &[Zone],
    totals: &[ZoneTotals],
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let mut csv = CsvWriter::create(
        path,
        "zone_id,label,population,population_on_barriers,population_covered,\
         population_covered_percent",
    )?;
    let mut order: Vec<usize> = (0..zones.len()).collect();
    order.sort_by(|&a, &b| compare_values(&zones[a].id, &zones[b].id));
    for (n, k) in order.into_iter().enumerate() {
        if n % 4096 == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        let (zone, t) = (&zones[k], &totals[k]);
        let percent = if t.population > 0.0 {
            format!("{}", t.covered / t.population * 100.0)
        } else {
            String::new()
        };
        csv.record(&[
            csv_field(&zone.id),
            zone.label
                .as_ref()
                .map(|l| csv_field(&serde_json::Value::String(l.clone())))
                .unwrap_or_default(),
            format!("{}", t.population),
            format!("{}", t.on_barriers),
            format!("{}", t.covered),
            percent,
        ])?;
    }
    csv.finish()
}

/// Geographic coverage.
#[derive(Debug)]
pub struct GeographicCoverage;

impl Analysis for GeographicCoverage {
    const NAME: &'static str = "geographic-coverage";
    const OUTPUTS: &'static [Output] = &[
        Output {
            file: CAPACITY_FILE,
            class: Some(DataClass::CoverageResult),
        },
        Output {
            file: RESIDUAL_FILE,
            class: Some(DataClass::PopulationResidual),
        },
        Output {
            file: CATCHMENTS_FILE,
            class: Some(DataClass::Catchment),
        },
        Output {
            file: BARRIERS_FILE,
            class: Some(DataClass::PopulationOnBarrier),
        },
        Output {
            file: ZONES_FILE,
            class: Some(DataClass::ZonalCoverage),
        },
    ];
    type Job = CoverageJob;
    type Prepared = PreparedCoverage;
    type Report = CoverageReport;

    fn prepare(
        job: CoverageJob,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<PreparedCoverage> {
        prepare(job, settings, monitor)
    }

    fn run(
        prepared: PreparedCoverage,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<(PathBuf, CoverageReport)> {
        run(prepared, settings, monitor)
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
