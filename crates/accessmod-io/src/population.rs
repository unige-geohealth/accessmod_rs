//! Adjust population distribution (schema version 1): move the population
//! that lies on barriers to the rest of its zone, or scale each zone to a
//! known population, as the AccessMod 5 toolbox does
//! (`modules/amPopulationBarrierCorrection`, rules in
//! `accessmod_core::population`).
//!
//! A barrier is a cell where the (merged) land cover is NoData. Zones are
//! burnt on the grid by cell centre; a cell in several zones belongs to the
//! first one in layer order.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::population::ZoneTotals;
use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use gdal::vector::LayerAccess;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Cancelled};
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::{JobFile, absolutize, check_files, check_schema_version};
use crate::output::{self, BLOCK, CsvWriter, FLOAT_NODATA, GeoTiff, Staging, csv_field};
use crate::raster::{
    AlignedBand, Window, for_each_window, integer_nodata, open_aligned_raster, open_class_raster,
    valid_value,
};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::vector::{Burn, VectorLayer, burn_layer, json_attribute, number_attribute};

/// Adjusted population file name.
pub const POPULATION_FILE: &str = "population_adjusted.tif";
/// Zone table file name.
pub const ZONES_FILE: &str = "zones.csv";

/// Zones a job may hold (zone indices are 32-bit, 0 is "no zone").
const MAX_ZONES: usize = u32::MAX as usize - 1;
/// Bytes held per zone: identifier, known population, totals (generous).
const BYTES_PER_ZONE: usize = 256;

/// Adjust population distribution job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdjustPopulationJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Digital elevation model; defines the grid. Its values are not used.
    pub dem: PathBuf,
    /// Population raster (people per cell), on the DEM lattice.
    pub population: PathBuf,
    /// Land cover (usually merged); NoData cells are barriers.
    pub land_cover: PathBuf,
    /// Zones within which population is redistributed.
    pub zones: ZoneSource,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

/// Zone polygons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZoneSource {
    /// Vector dataset with polygons.
    pub path: PathBuf,
    /// Layer name; the first layer when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Field holding a unique zone identifier.
    pub id_field: String,
    /// Optional OGR SQL attribute filter.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Field holding the known population of each zone; when omitted, each
    /// zone keeps the population of the raster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub population_field: Option<String>,
}

impl JobFile for AdjustPopulationJob {
    fn resolve_paths(mut self, base: &Path) -> Self {
        for path in [
            &mut self.dem,
            &mut self.population,
            &mut self.land_cover,
            &mut self.zones.path,
            &mut self.output_dir,
        ] {
            absolutize(path, base);
        }
        self
    }
}

impl AdjustPopulationJob {
    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues: Vec<String> = check_schema_version(self.schema_version)
            .into_iter()
            .collect();
        if self.zones.id_field.trim().is_empty() {
            issues.push("zones.id_field is empty".into());
        }
        if self
            .zones
            .population_field
            .as_ref()
            .is_some_and(|f| f.trim().is_empty())
        {
            issues.push("zones.population_field is empty".into());
        }
        issues
    }
}

/// A zone read from the layer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Zone {
    /// Identifier.
    pub id: serde_json::Value,
    /// Known population, when `population_field` is set.
    pub known: Option<f64>,
}

/// Inputs validated, ready to adjust.
#[derive(Debug)]
pub struct PreparedPopulation {
    /// Job with resolved paths.
    pub job: AdjustPopulationJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Zones in layer order (zone index = position + 1).
    pub zones: Vec<Zone>,
    /// Layer read.
    pub zones_layer: String,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Input files.
    pub fingerprints: Vec<Fingerprint>,
    /// Seconds spent per phase.
    pub durations_s: std::collections::BTreeMap<&'static str, f64>,
}

/// Validate the job against its inputs and read the zones.
pub fn prepare(
    job: AdjustPopulationJob,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedPopulation> {
    let started = Instant::now();
    let mut issues = job.check();
    issues.extend(check_files(
        &job.output_dir,
        [
            ("dem".to_owned(), job.dem.as_path()),
            ("population".to_owned(), job.population.as_path()),
            ("land_cover".to_owned(), job.land_cover.as_path()),
            ("zones.path".to_owned(), job.zones.path.as_path()),
        ],
    ));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let (_, grid) = GridSpec::open_dem(&job.dem)?;
    let plan = population_plan(&grid, settings, 0)?;
    plan.configure_gdal()?;
    for result in [
        open_aligned_raster(&job.population, &grid, "population").map(drop),
        open_class_raster(&job.land_cover, &grid, "land cover").map(drop),
    ] {
        if let Err(Error::Validation { issues: more }) = result {
            issues.extend(more);
        } else {
            result?;
        }
    }
    let (zones_layer, zones) = match read_zones(&job.zones, &grid, cancelled) {
        Ok(read) => read,
        Err(Error::Validation { issues: more }) => {
            issues.extend(more);
            (String::new(), Vec::new())
        }
        Err(other) => return Err(other),
    };
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let mut warnings = Vec::new();
    if zones.is_empty() {
        warnings.push(Warning {
            code: "no_zone",
            message: "the zone layer has no feature: every cell is outside the zones".into(),
        });
    }
    let plan = population_plan(&grid, settings, zones.len())?;
    let mut durations_s = std::collections::BTreeMap::new();
    durations_s.insert("validate", started.elapsed().as_secs_f64());
    let started = Instant::now();
    let fingerprints = fingerprints(
        &[
            ("dem", job.dem.as_path()),
            ("population", job.population.as_path()),
            ("land_cover", job.land_cover.as_path()),
            ("zones", job.zones.path.as_path()),
        ],
        cancelled,
    )?;
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());
    Ok(PreparedPopulation {
        job,
        grid,
        zones,
        zones_layer,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// Memory plan: the zone index grid (4 bytes per cell), population, land
/// cover and output blocks, and `zones` zone entries.
fn population_plan(
    grid: &GridSpec,
    settings: &ExecutionSettings,
    zones: usize,
) -> Result<ResourcePlan> {
    let geometry = TileGeometry::new(grid.rows, grid.cols, accessmod_core::store::DEFAULT_TILE)?;
    let per_tile = TiledGrid::<u32>::tile_bytes(geometry) as u64;
    // Per block: population (f64), land cover (i64), zone row and output.
    let buffer_bytes = (BLOCK * BLOCK * (8 + 8 + 8) + BLOCK * 4 + zones * BYTES_PER_ZONE) as u64;
    ResourcePlan::for_tiles(grid.rows, grid.cols, settings, per_tile, buffer_bytes, 0)
}

/// Zones in layer order, with unique identifiers and, when asked, a known
/// population.
fn read_zones(
    source: &ZoneSource,
    grid: &GridSpec,
    cancelled: &dyn Fn() -> bool,
) -> Result<(String, Vec<Zone>)> {
    let vector = VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        "zones",
    )?;
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let id_index = vector.field_index(&layer, &source.id_field)?;
    let known_index = match &source.population_field {
        Some(field) => Some(vector.field_index(&layer, field)?),
        None => None,
    };
    let mut zones = Vec::new();
    let mut issues = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, feature) in layer.features().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let which = feature
            .fid()
            .map_or_else(|| format!("#{index}"), |fid| format!("FID {fid}"));
        let id = json_attribute(feature.field(id_index).map_err(Error::gdal(&source.path))?);
        let Some(id) = id else {
            issues.push(format!(
                "zones feature {which}: empty {:?}",
                source.id_field
            ));
            continue;
        };
        if !seen.insert(id.to_string()) {
            issues.push(format!(
                "zones feature {which}: identifier {id} is not unique"
            ));
        }
        let known = match known_index {
            Some(i) => {
                let value = feature
                    .field(i)
                    .map_err(Error::gdal(&source.path))?
                    .and_then(number_attribute);
                match value {
                    Some(v) if v.is_finite() && v >= 0.0 => Some(v),
                    _ => {
                        issues.push(format!(
                            "zones feature {which} ({id}): {:?} must be a number >= 0",
                            source.population_field.as_deref().unwrap_or_default()
                        ));
                        None
                    }
                }
            }
            None => None,
        };
        zones.push(Zone { id, known });
    }
    if zones.len() > MAX_ZONES {
        issues.push(format!("more than {MAX_ZONES} zones"));
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    Ok((vector.name, zones))
}

/// One row of `zones.csv`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ZoneResult {
    /// Identifier.
    pub id: serde_json::Value,
    /// Cells of the zone holding population.
    pub cells: u64,
    /// Population before adjustment.
    pub population: f64,
    /// Population on barriers before adjustment.
    pub population_on_barrier: f64,
    /// Known population, if given.
    pub population_known: Option<f64>,
    /// Scale factor of the cells off barriers; `None` when the zone has no
    /// population off barriers (its population cannot be placed).
    pub ratio: Option<f64>,
    /// Population after adjustment.
    pub population_adjusted: f64,
}

/// Totals of the adjustment.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PopulationSummary {
    /// Population of the raster inside the grid.
    pub population: f64,
    /// Population after adjustment.
    pub population_adjusted: f64,
    /// Population outside every zone (no output there).
    pub population_outside_zones: f64,
    /// Population on barriers before adjustment.
    pub population_on_barrier: f64,
    /// Population of zones without population off barriers (not placed).
    pub population_unplaced: f64,
    /// Zones.
    pub zones: usize,
    /// Zones holding no populated cell.
    pub zones_without_population: usize,
    /// Cells inside more than one zone (kept in the first).
    pub overlapping_cells: u64,
}

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct PopulationRecord {
    /// Job with resolved paths and defaults.
    pub job: AdjustPopulationJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// DEM grid.
    pub grid: GridReport,
    /// Totals.
    pub result: PopulationSummary,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Execution details of a population adjustment.
#[derive(Debug, Clone, Serialize)]
pub struct PopulationStore {
    /// Cache counters of the zone grid.
    pub store: StoreStats,
}

/// `run.json` of a population adjustment.
pub type PopulationReport = Report<PopulationRecord, PopulationStore>;

/// Deviations from AccessMod 5 that apply to every adjustment.
pub fn deviations() -> Vec<Deviation> {
    vec![
        Deviation {
            code: "population_on_lattice",
            message: "The population raster must lie on the DEM lattice. AccessMod 5 \
                      resamples rasters when importing them.",
        },
        Deviation {
            code: "overlapping_zones",
            message: "A cell in several zones belongs to the first zone in layer order. \
                      AccessMod 5 imports zones into a GRASS topology, which splits overlaps \
                      into areas of their own.",
        },
    ]
}

/// The population inputs read block by block.
struct Inputs<'a> {
    population: AlignedBand<'a>,
    population_nodata: Option<f64>,
    land_cover: AlignedBand<'a>,
    land_cover_nodata: Option<i64>,
}

/// Values of one cell of a block: population (if any), whether it is a
/// barrier, its zone index (0: none).
type CellVisit<'v> = dyn FnMut(usize, usize, Option<f64>, bool, u32) -> Result<()> + 'v;

impl Inputs<'_> {
    /// Visit every cell of block `rows × cols`.
    fn visit_block(
        &self,
        rows: std::ops::Range<usize>,
        cols: std::ops::Range<usize>,
        zones: &mut TiledGrid<u32>,
        zone_row: &mut [u32],
        visit: &mut CellVisit<'_>,
    ) -> Result<()> {
        let population: Option<Window<f64>> = self.population.read(rows.clone(), cols.clone())?;
        let land_cover: Option<Window<i64>> = self.land_cover.read(rows.clone(), cols.clone())?;
        let zone_row = &mut zone_row[..cols.len()];
        for row in rows {
            zones.read_row(row, cols.start, zone_row)?;
            for (k, col) in cols.clone().enumerate() {
                let people = population
                    .as_ref()
                    .and_then(|w| w.get(row, col))
                    .and_then(|v| valid_value(v, self.population_nodata));
                let class = land_cover.as_ref().and_then(|w| w.get(row, col));
                let barrier = class.is_none_or(|c| Some(c) == self.land_cover_nodata);
                visit(row, col, people, barrier, zone_row[k])?;
            }
        }
        Ok(())
    }
}

/// Adjust the population and write every output atomically.
pub fn run(
    mut prepared: PreparedPopulation,
    settings: &ExecutionSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<(PathBuf, PopulationReport)> {
    let job = &prepared.job;
    let grid = &prepared.grid;
    let staging = Staging::create(&job.output_dir)?;
    let geometry = TileGeometry::new(grid.rows, grid.cols, prepared.plan.tile)?;
    let mut zones = TiledGrid::new(
        geometry,
        0u32,
        prepared.plan.resident_tiles,
        &settings.tmp_dir,
        "zones",
    )?;

    // Zones on the grid, by cell centre; the first zone keeps a shared cell.
    let started = Instant::now();
    let vector = VectorLayer::open(
        &job.zones.path,
        job.zones.layer.as_deref(),
        job.zones.filter.as_deref(),
        grid,
        "zones",
    )?;
    let overlaps = Cell::new(0u64);
    let mut next = 0u32;
    burn_layer(&vector, grid, &mut zones, Burn::Polygons, cancelled, |_| {
        next += 1;
        let zone = next;
        let overlaps = &overlaps;
        Ok(Some(move |cell: &mut u32| {
            if *cell == 0 {
                *cell = zone;
            } else if *cell != zone {
                overlaps.set(overlaps.get() + 1);
            }
        }))
    })?;
    prepared
        .durations_s
        .insert("zones", started.elapsed().as_secs_f64());

    let (population_ds, population_alignment) =
        open_aligned_raster(&job.population, grid, "population")?;
    let (land_cover_ds, land_cover_alignment) =
        open_class_raster(&job.land_cover, grid, "land cover")?;
    let population = AlignedBand::new(&population_ds, &job.population, population_alignment, grid)?;
    let land_cover = AlignedBand::new(&land_cover_ds, &job.land_cover, land_cover_alignment, grid)?;
    let inputs = Inputs {
        population_nodata: population.nodata(),
        land_cover_nodata: integer_nodata(land_cover.nodata()),
        population,
        land_cover,
    };
    let mut zone_row = vec![0u32; BLOCK];

    // Pass 1: totals per zone.
    let started = Instant::now();
    let mut summary = PopulationSummary {
        zones: prepared.zones.len(),
        overlapping_cells: overlaps.get(),
        ..PopulationSummary::default()
    };
    let mut totals = vec![ZoneTotals::default(); prepared.zones.len() + 1];
    for_each_window(
        0..grid.rows,
        0..grid.cols,
        (BLOCK, BLOCK),
        cancelled,
        |rows, cols| {
            inputs.visit_block(
                rows,
                cols,
                &mut zones,
                &mut zone_row,
                &mut |_, _, people, barrier, zone| {
                    let Some(people) = people else {
                        return Ok(());
                    };
                    summary.population += people;
                    if barrier {
                        summary.population_on_barrier += people;
                    }
                    if zone == 0 {
                        summary.population_outside_zones += people;
                    } else {
                        totals[zone as usize].add(people, barrier);
                    }
                    Ok(())
                },
            )
        },
    )?;
    let ratios: Vec<Option<f64>> = std::iter::once(None)
        .chain(
            prepared
                .zones
                .iter()
                .zip(&totals[1..])
                .map(|(zone, total)| total.ratio(zone.known)),
        )
        .collect();
    prepared
        .durations_s
        .insert("totals", started.elapsed().as_secs_f64());

    // Pass 2: adjusted population.
    let started = Instant::now();
    let tif = GeoTiff::create::<f64>(
        &staging.path().join(POPULATION_FILE),
        grid,
        3,
        settings.threads,
        FLOAT_NODATA,
    )?;
    let mut adjusted = vec![0.0f64; prepared.zones.len() + 1];
    let mut block = Vec::with_capacity(BLOCK * BLOCK);
    for_each_window(
        0..grid.rows,
        0..grid.cols,
        (BLOCK, BLOCK),
        cancelled,
        |rows, cols| {
            inputs.visit_block(
                rows.clone(),
                cols.clone(),
                &mut zones,
                &mut zone_row,
                &mut |_, _, people, barrier, zone| {
                    let value = match (people, barrier, ratios[zone as usize]) {
                        (Some(people), false, Some(ratio)) => {
                            let value = people * ratio;
                            adjusted[zone as usize] += value;
                            value
                        }
                        _ => FLOAT_NODATA,
                    };
                    block.push(value);
                    Ok(())
                },
            )?;
            tif.write(rows, cols, &mut block)
        },
    )?;
    tif.close()?;

    let mut csv = CsvWriter::create(
        &staging.path().join(ZONES_FILE),
        "zone_id,cells,population,population_on_barrier,population_known,ratio,\
         population_adjusted",
    )?;
    let number = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_default();
    for (k, zone) in prepared.zones.iter().enumerate() {
        let total = &totals[k + 1];
        let ratio = ratios[k + 1];
        summary.population_adjusted += adjusted[k + 1];
        if total.cells == 0 {
            summary.zones_without_population += 1;
        }
        if ratio.is_none() {
            summary.population_unplaced += zone.known.unwrap_or(total.population);
        }
        let row = ZoneResult {
            id: zone.id.clone(),
            cells: total.cells,
            population: total.population,
            population_on_barrier: total.on_barrier(),
            population_known: zone.known,
            ratio,
            population_adjusted: adjusted[k + 1],
        };
        csv.record(&[
            csv_field(&row.id),
            row.cells.to_string(),
            number(Some(row.population)),
            number(Some(row.population_on_barrier)),
            number(row.population_known),
            number(row.ratio),
            number(Some(row.population_adjusted)),
        ])?;
    }
    csv.finish()?;
    prepared
        .durations_s
        .insert("write_outputs", started.elapsed().as_secs_f64());

    let mut warnings = prepared.warnings.clone();
    if summary.population_outside_zones > 0.0 {
        warnings.push(Warning {
            code: "population_outside_zones",
            message: format!(
                "{} people lie outside every zone and are left out",
                summary.population_outside_zones
            ),
        });
    }
    if summary.population_unplaced > 0.0 {
        warnings.push(Warning {
            code: "population_unplaced",
            message: format!(
                "{} people belong to zones with no population off barriers and cannot be \
                 placed (see ratio in {ZONES_FILE})",
                summary.population_unplaced
            ),
        });
    }
    if summary.overlapping_cells > 0 {
        warnings.push(Warning {
            code: "overlapping_zones",
            message: format!(
                "{} cells lie in several zones; each belongs to the first zone in layer order",
                summary.overlapping_cells
            ),
        });
    }
    let store = zones.stats();
    let PreparedPopulation {
        job,
        grid,
        plan,
        fingerprints,
        durations_s,
        ..
    } = prepared;
    let report = Report::new(
        PopulationRecord {
            job,
            inputs: fingerprints,
            grid: GridReport::of(&grid),
            result: summary,
            warnings,
            deviations: deviations(),
        },
        Execution::now(settings, plan, durations_s, PopulationStore { store }),
    );
    let output_dir = output::finish(staging, &report.run.job, &report, cancelled)?;
    Ok((output_dir, report))
}

/// Adjust population distribution.
#[derive(Debug)]
pub struct AdjustPopulation;

impl Analysis for AdjustPopulation {
    type Job = AdjustPopulationJob;
    type Prepared = PreparedPopulation;
    type Report = PopulationReport;

    fn prepare(
        job: AdjustPopulationJob,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<PreparedPopulation> {
        prepare(job, settings, cancelled)
    }

    fn run(
        prepared: PreparedPopulation,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<(PathBuf, PopulationReport)> {
        run(prepared, settings, cancelled)
    }

    fn warnings(prepared: &PreparedPopulation) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &PreparedPopulation) -> String {
        format!(
            "adjust population: {} zones on a {} x {} grid{}",
            prepared.zones.len(),
            prepared.grid.cols,
            prepared.grid.rows,
            if prepared.job.zones.population_field.is_some() {
                ", known populations"
            } else {
                ""
            }
        )
    }

    fn validation(prepared: &PreparedPopulation) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "zones_layer": prepared.zones_layer,
            "zones": prepared.zones,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &PopulationReport) -> String {
        let r = &report.run.result;
        format!(
            "population {} -> {} ({} on barriers moved, {} outside zones)",
            r.population,
            r.population_adjusted,
            r.population_on_barrier,
            r.population_outside_zones
        )
    }
}
