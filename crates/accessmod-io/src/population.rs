//! Adjust population distribution (schema version 1): move the population
//! that lies on barriers to the rest of its zone, or scale each zone to a
//! known population, as the AccessMod 5 toolbox does
//! (`modules/amPopulationBarrierCorrection`, rules in
//! `accessmod_core::population`).
//!
//! A barrier is a cell where the (merged) land cover is NoData. Zones are
//! burnt on the grid by cell centre; a cell in several zones belongs to the
//! first one in layer order.

use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::population::{Target, ZoneTotals};
use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use gdal::vector::LayerAccess;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Cancelled};
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::{JobFile, absolutize, check_files, check_schema_version};
use crate::output::{self, BLOCK, CsvWriter, FLOAT_NODATA, GeoTiff, Staging, csv_field};
use crate::raster::{AlignedBand, for_each_block, open_aligned_raster, open_class_raster};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::vector::{VectorLayer, burn_zones, json_attribute, number_attribute};

/// Adjusted population file name.
pub const POPULATION_FILE: &str = "population_adjusted.tif";
/// Zone table file name.
pub const ZONES_FILE: &str = "zones.csv";

/// Zones a job may hold (zone indices are 32-bit, 0 is "no zone").
const MAX_ZONES: usize = u32::MAX as usize - 1;

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
    /// Field holding the zone identifier, written to `zones.csv`.
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

/// Inputs validated, ready to adjust.
#[derive(Debug)]
pub struct PreparedPopulation {
    /// Job with resolved paths.
    pub job: AdjustPopulationJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Target of each zone, in layer order (zone index = position + 1).
    /// Identifiers are read again when writing `zones.csv`, so that memory
    /// does not grow with their length.
    pub targets: Vec<Target>,
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

/// Validate the job against its inputs and read the zone targets.
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
    population_plan(&grid, settings, 0)?.configure_gdal()?;
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
    // Plan with the zone count before reading the zones.
    let plan = population_plan(&grid, settings, count)?;
    let (targets, missing) = read_targets(&vector, &job.zones, cancelled)?;
    let mut warnings = Vec::new();
    if targets.is_empty() {
        warnings.push(Warning {
            code: "no_zone",
            message: "the zone layer has no feature: every cell is outside the zones".into(),
        });
    }
    if missing > 0 {
        warnings.push(Warning {
            code: "known_population_missing",
            message: format!(
                "{missing} zones have no value in {:?}: their population is left out",
                job.zones.population_field.as_deref().unwrap_or_default()
            ),
        });
    }
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
        targets,
        zones_layer: vector.name,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// Memory plan: the zone index grid (4 bytes per cell), population, land
/// cover and output blocks, and `zones` zone entries (target, totals,
/// ratio, adjusted sum). A polygon's vertices are not counted (see
/// `vector::for_each_part`).
fn population_plan(
    grid: &GridSpec,
    settings: &ExecutionSettings,
    zones: usize,
) -> Result<ResourcePlan> {
    let geometry = TileGeometry::new(grid.rows, grid.cols, accessmod_core::store::DEFAULT_TILE)?;
    let per_tile = TiledGrid::<u32>::tile_bytes(geometry) as u64;
    let per_zone = std::mem::size_of::<Target>()
        + std::mem::size_of::<ZoneTotals>()
        + std::mem::size_of::<Option<f64>>()
        + std::mem::size_of::<f64>();
    // Per block: two bands read as f64 then held as Option<f64>, zones and
    // output.
    let buffer_bytes = (BLOCK * BLOCK * (2 * (8 + 16) + 4 + 8) + zones * per_zone) as u64;
    ResourcePlan::for_tiles(grid.rows, grid.cols, settings, per_tile, buffer_bytes, 0)
}

fn open_zones(source: &ZoneSource, grid: &GridSpec) -> Result<VectorLayer> {
    VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        "zones",
    )
}

/// Target of each zone, in layer order, and the number of zones whose
/// known population is missing. Identifiers must be set; known populations,
/// when present, must be numbers >= 0.
fn read_targets(
    vector: &VectorLayer,
    source: &ZoneSource,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<Target>, usize)> {
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let id_index = vector.field_index(&layer, &source.id_field)?;
    let known_index = match &source.population_field {
        Some(field) => Some(vector.field_index(&layer, field)?),
        None => None,
    };
    let mut targets = Vec::new();
    let mut missing = 0;
    let mut issues = Vec::new();
    for (index, feature) in layer.features().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let which = feature
            .fid()
            .map_or_else(|| format!("#{index}"), |fid| format!("FID {fid}"));
        if feature
            .field(id_index)
            .map_err(Error::gdal(&source.path))?
            .is_none()
        {
            issues.push(format!(
                "zones feature {which}: empty {:?}",
                source.id_field
            ));
        }
        let target = match known_index {
            None => Target::Zone,
            Some(i) => match feature.field(i).map_err(Error::gdal(&source.path))? {
                None => {
                    missing += 1;
                    Target::Missing
                }
                Some(value) => match number_attribute(value) {
                    Some(v) if v.is_finite() && v >= 0.0 => Target::Known(v),
                    _ => {
                        issues.push(format!(
                            "zones feature {which}: {:?} must be a number >= 0",
                            source.population_field.as_deref().unwrap_or_default()
                        ));
                        Target::Missing
                    }
                },
            },
        };
        targets.push(target);
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    Ok((targets, missing))
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
            code: "population_summary",
            message: "Totals count population outside every zone, on barriers (land cover \
                      NoData) and unplaced (zones without a ratio) separately. AccessMod 5 \
                      counts zones without a ratio as outside, and derives the barrier total \
                      from the other two.",
        },
        Deviation {
            code: "overlapping_zones",
            message: "A cell in several zones belongs to the first zone in layer order. \
                      AccessMod 5 imports zones into a GRASS topology, which splits overlaps \
                      into areas of their own.",
        },
    ]
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
    let vector = open_zones(&job.zones, grid)?;
    let mut next = 0u32;
    let (_, overlaps) = burn_zones(&vector, grid, &mut zones, cancelled, |_| {
        next += 1;
        Ok(Some(next))
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
    // Band 0: people per cell; band 1: land cover (`None`: barrier).
    let bands = [&population, &land_cover];

    // Pass 1: totals per zone.
    let started = Instant::now();
    let mut summary = PopulationSummary {
        zones: prepared.targets.len(),
        overlapping_cells: overlaps,
        ..PopulationSummary::default()
    };
    let mut totals = vec![ZoneTotals::default(); prepared.targets.len() + 1];
    for_each_block(
        grid,
        (BLOCK, BLOCK),
        &bands,
        Some(&mut zones),
        cancelled,
        |block| {
            for i in 0..block.len() {
                let Some(people) = block.value(0, i) else {
                    continue;
                };
                let barrier = block.value(1, i).is_none();
                summary.population += people;
                if barrier {
                    summary.population_on_barrier += people;
                }
                match block.zone(i) {
                    0 => summary.population_outside_zones += people,
                    zone => totals[zone as usize].add(people, barrier),
                }
            }
            Ok(())
        },
    )?;
    let ratios: Vec<Option<f64>> = std::iter::once(None)
        .chain(
            prepared
                .targets
                .iter()
                .zip(&totals[1..])
                .map(|(target, total)| total.ratio(*target)),
        )
        .collect();
    prepared
        .durations_s
        .insert("totals", started.elapsed().as_secs_f64());

    // Pass 2: adjusted population, written block by block.
    let started = Instant::now();
    let tif = GeoTiff::create::<f64>(
        &staging.path().join(POPULATION_FILE),
        grid,
        3,
        settings.threads,
        FLOAT_NODATA,
    )?;
    let mut adjusted = vec![0.0f64; prepared.targets.len() + 1];
    let mut values = Vec::with_capacity(BLOCK * BLOCK);
    for_each_block(
        grid,
        (BLOCK, BLOCK),
        &bands,
        Some(&mut zones),
        cancelled,
        |block| {
            for i in 0..block.len() {
                let zone = block.zone(i) as usize;
                let value = match (block.value(0, i), block.value(1, i), ratios[zone]) {
                    (Some(people), Some(_), Some(ratio)) => {
                        let value = people * ratio;
                        adjusted[zone] += value;
                        value
                    }
                    _ => FLOAT_NODATA,
                };
                values.push(value);
            }
            tif.write(block.rows.clone(), block.cols.clone(), &mut values)
        },
    )?;
    tif.close()?;

    let mut csv = CsvWriter::create(
        &staging.path().join(ZONES_FILE),
        "zone_id,cells,population,population_on_barrier,population_known,ratio,\
         population_adjusted",
    )?;
    let number = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_default();
    // Identifiers are read again, in the same order as the targets.
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let id_index = vector.field_index(&layer, &job.zones.id_field)?;
    let mut zone = 0usize;
    for feature in layer.features() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        zone += 1;
        let (Some(target), Some(total)) = (prepared.targets.get(zone - 1), totals.get(zone)) else {
            return Err(vector.invalid("the zone layer changed during the run".into()));
        };
        let id = json_attribute(
            feature
                .field(id_index)
                .map_err(Error::gdal(&job.zones.path))?,
        )
        .unwrap_or_default();
        summary.population_adjusted += adjusted[zone];
        summary.population_unplaced += total.unplaced(*target);
        // As v.rast.stats: a zone without population cells has no sums.
        let sums = (total.cells > 0).then_some(total);
        if sums.is_none() {
            summary.zones_without_population += 1;
        }
        csv.record(&[
            csv_field(&id),
            total.cells.to_string(),
            number(sums.map(|t| t.population)),
            number(sums.map(ZoneTotals::on_barrier)),
            number(match target {
                Target::Known(population) => Some(*population),
                Target::Zone | Target::Missing => None,
            }),
            number(ratios[zone]),
            number(sums.map(|_| adjusted[zone])),
        ])?;
    }
    if zone != prepared.targets.len() {
        return Err(vector.invalid("the zone layer changed during the run".into()));
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
            prepared.targets.len(),
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
            "zones": prepared.targets.len(),
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
