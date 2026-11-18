//! Merge land cover (schema version 1): a stack of land-cover rasters, road
//! layers and barrier layers merged on the DEM lattice, as the AccessMod 5
//! toolbox does (`modules/amAnalysisMergeLandCover`).
//!
//! Validation (`prepare`) reads every item once to list its classes and
//! refuses class conflicts; execution (`run`) paints the items into a tiled
//! grid by priority (`accessmod_core::merge`) and streams the outputs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::merge::{MergeCell, bridge_row};
use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Monitor, Output};
use crate::class::DataClass;
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::{JobFile, check_files, check_schema_version, repeated};
use crate::output::{self, BLOCK, CsvWriter, GeoTiff, Staging, csv_field};
use crate::raster::{
    AlignedBand, CHUNK_COLS, STRIP_ROWS, for_each_window, integer_nodata, open_class_raster,
};
use crate::report::{Deviation, Execution, Fingerprint, GridReport, Report, Warning, fingerprints};
use crate::resources::{ExecutionSettings, ResourcePlan};
use crate::vector::{Burn, FeatureCounts, VectorLayer, burn_layer, integer_attribute};

/// Merged land cover file name.
pub const MERGED_FILE: &str = "land_cover_merged.tif";
/// Bridge map file name (written when bridges are cleaned).
pub const BRIDGES_FILE: &str = "bridges.tif";
/// Class table file name.
pub const CLASSES_FILE: &str = "classes.csv";
/// NoData of the merged land cover (Int32).
pub const MERGED_NODATA: i32 = i32::MIN;
/// NoData of the bridge map (Byte); bridge cells are 1.
pub const BRIDGE_NODATA: u8 = 0;
/// AccessMod 5 adds this to road classes below it so that they do not
/// collide with land-cover classes (`checkDontAdd1000` unchecked).
pub const ROAD_CLASS_OFFSET: i64 = 1000;

/// Distinct classes a raster item may hold.
const MAX_ITEM_CLASSES: usize = 65_536;

fn yes() -> bool {
    true
}

/// Merge land cover job (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeLandCoverJob {
    /// Must be 1.
    pub schema_version: u32,
    /// Digital elevation model; defines the grid (CRS, resolution, origin,
    /// extent). Its values are not used.
    #[schemars(extend("x-accessmod-class" = ["rDem"]))]
    pub dem: PathBuf,
    /// Stack items, highest priority first. A cell takes the class of the
    /// first item covering it, unless a barrier above that item covers it:
    /// a barrier empties the items below it (at the top, every item; below
    /// the roads, only what is under them, so roads cross it).
    pub stack: Vec<StackItem>,
    /// Remove one-cell diagonal bridges created by roads across barriers
    /// and write `bridges.tif` (AccessMod 5 default: on).
    #[serde(default = "yes")]
    pub clean_bridges: bool,
    /// Output directory; must not exist or be empty.
    pub output_dir: PathBuf,
}

/// One item of the stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StackItem {
    /// Raster of integer classes on the DEM lattice (stack land cover).
    LandCover(LandCoverItem),
    /// Road lines with a class attribute (stack road).
    Road(RoadItem),
    /// Barrier lines or polygons (stack barrier): it empties the items
    /// below it in the stack.
    Barrier(BarrierItem),
}

/// Land-cover raster item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LandCoverItem {
    /// Raster with integer classes, on the DEM lattice and in its CRS.
    #[schemars(extend("x-accessmod-class" = ["rLandCover"]))]
    pub path: PathBuf,
    /// Labels of the raster's classes (before `reclass`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<ClassLabel>,
    /// Class changes applied while merging, e.g. to resolve a conflict
    /// (AccessMod 5 "quick correction").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reclass: Vec<Reclass>,
}

/// Road layer item. Each class is a stack item of its own in AccessMod 5;
/// here the order of `classes` sets their priority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoadItem {
    /// Vector dataset with line features.
    #[schemars(extend("x-accessmod-class" = ["vRoad"]))]
    pub path: PathBuf,
    /// Layer name; the first layer when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Integer field holding the road class.
    pub class_field: String,
    /// Field holding the class label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_field: Option<String>,
    /// Optional OGR SQL attribute filter, e.g. `"surface = 'paved'"`.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Classes to use (values of `class_field`), highest priority first;
    /// all classes in ascending order when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classes: Option<Vec<i64>>,
    /// Add 1000 to classes below 1000, as AccessMod 5 does by default.
    #[serde(default = "yes")]
    pub add_1000: bool,
}

/// Barrier layer item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BarrierItem {
    /// Vector dataset with line or polygon features.
    #[schemars(extend("x-accessmod-class" = ["vBarrier"]))]
    pub path: PathBuf,
    /// Layer name; the first layer when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Optional OGR SQL attribute filter.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// Label of a class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassLabel {
    /// Class value.
    pub class: i64,
    /// Label.
    pub label: String,
}

/// Class change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reclass {
    /// Class in the raster.
    pub from: i64,
    /// Class in the merged land cover.
    pub to: i64,
}

impl StackItem {
    fn path(&self) -> &Path {
        match self {
            Self::LandCover(item) => &item.path,
            Self::Road(item) => &item.path,
            Self::Barrier(item) => &item.path,
        }
    }

    fn path_mut(&mut self) -> &mut PathBuf {
        match self {
            Self::LandCover(item) => &mut item.path,
            Self::Road(item) => &mut item.path,
            Self::Barrier(item) => &mut item.path,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::LandCover(_) => "land_cover",
            Self::Road(_) => "road",
            Self::Barrier(_) => "barrier",
        }
    }
}

impl JobFile for MergeLandCoverJob {
    fn for_each_path(&mut self, f: &mut dyn FnMut(&mut PathBuf)) {
        f(&mut self.dem);
        f(&mut self.output_dir);
        for item in &mut self.stack {
            f(item.path_mut());
        }
    }
}

impl MergeLandCoverJob {
    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues: Vec<String> = check_schema_version(self.schema_version)
            .into_iter()
            .collect();
        if !self
            .stack
            .iter()
            .any(|item| !matches!(item, StackItem::Barrier(_)))
        {
            issues.push("stack has no land_cover or road item".into());
        }
        for (i, item) in self.stack.iter().enumerate() {
            let at = format!("stack item {} ({})", i + 1, item.kind());
            match item {
                StackItem::LandCover(item) => {
                    duplicates(
                        item.labels.iter().map(|l| l.class),
                        &at,
                        "labels",
                        &mut issues,
                    );
                    duplicates(
                        item.reclass.iter().map(|r| r.from),
                        &at,
                        "reclass",
                        &mut issues,
                    );
                }
                StackItem::Road(item) => {
                    if item.class_field.trim().is_empty() {
                        issues.push(format!("{at}: class_field is empty"));
                    }
                    if let Some(classes) = &item.classes {
                        if classes.is_empty() {
                            issues.push(format!("{at}: classes is empty"));
                        } else if classes.len() > MAX_ITEM_CLASSES {
                            issues.push(format!(
                                "{at}: classes lists more than {MAX_ITEM_CLASSES} classes"
                            ));
                        }
                        duplicates(classes.iter().copied(), &at, "classes", &mut issues);
                    }
                }
                StackItem::Barrier(_) => {}
            }
        }
        issues
    }
}

fn duplicates(values: impl Iterator<Item = i64>, at: &str, field: &str, issues: &mut Vec<String>) {
    for (value, _, _) in repeated(values) {
        issues.push(format!("{at}: class {value} is listed twice in {field}"));
    }
}

/// A class an item contributes to the merged land cover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemClass {
    /// Class in the merged land cover.
    pub class: i32,
    /// Class in the item (raster value or attribute value).
    pub source_class: i64,
    /// Label, if known.
    pub label: Option<String>,
    /// Priority in the stack (lower wins).
    pub rank: u32,
    /// Raster cells inside the grid, or road features.
    pub count: u64,
}

/// Validated stack item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemPlan {
    /// 1-based position in the stack.
    pub position: usize,
    /// `land_cover`, `road` or `barrier`.
    pub kind: &'static str,
    /// Dataset path.
    pub path: PathBuf,
    /// Layer read (vector items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Whether geometries are reprojected to the DEM CRS (vector items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reprojected: Option<bool>,
    /// Classes contributed, highest priority first.
    pub classes: Vec<ItemClass>,
    /// Feature counts, filled while merging (vector items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<FeatureCounts>,
    /// Priority of a barrier: it empties the items of a higher rank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub barrier_rank: Option<u32>,
}

/// Inputs validated, ready to merge.
#[derive(Debug)]
pub struct PreparedMerge {
    /// Job with resolved paths.
    pub job: MergeLandCoverJob,
    /// DEM grid.
    pub grid: GridSpec,
    /// Stack items, in job order.
    pub items: Vec<ItemPlan>,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Input files.
    pub fingerprints: Vec<Fingerprint>,
    /// Seconds spent per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
}

/// Validate the job against its inputs: open every item, list its classes
/// and check for class conflicts. Fails with every problem found.
pub fn prepare(
    job: MergeLandCoverJob,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<PreparedMerge> {
    let cancelled = monitor.cancelled();
    monitor.stage("validate");
    let started = Instant::now();
    let mut issues = job.check();
    let stack_files = job.stack.iter().enumerate().map(|(i, item)| {
        (
            format!("stack item {} ({})", i + 1, item.kind()),
            item.path(),
        )
    });
    issues.extend(check_files(
        &job.output_dir,
        std::iter::once(("dem".to_owned(), job.dem.as_path())).chain(stack_files),
    ));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }

    let (_, grid) = GridSpec::open_dem(&job.dem)?;
    // The GDAL cache depends on the budget only: set it before reading items.
    let plan = merge_plan(&grid, settings, 0)?;
    plan.configure_gdal()?;

    let mut warnings = Vec::new();
    let mut items = Vec::with_capacity(job.stack.len());
    let mut rank = 0u32;
    for (i, item) in job.stack.iter().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let position = i + 1;
        let at = format!("stack item {position} ({})", item.kind());
        let planned = match item {
            StackItem::LandCover(item) => plan_land_cover(
                item,
                &grid,
                &at,
                position,
                &mut rank,
                &mut warnings,
                cancelled,
            ),
            StackItem::Road(item) => plan_road(
                item,
                &grid,
                &at,
                position,
                &mut rank,
                &mut warnings,
                cancelled,
            ),
            StackItem::Barrier(item) => {
                // Its own rank, between the items above and below it.
                let barrier_rank = rank;
                rank += 1;
                plan_barrier(item, &grid, &at, position, &mut warnings).map(|mut plan| {
                    plan.barrier_rank = Some(barrier_rank);
                    plan
                })
            }
        };
        match planned {
            Ok(planned) => items.push(planned),
            Err(Error::Validation { issues: more }) => issues.extend(more),
            Err(other) => return Err(other),
        }
    }
    issues.extend(class_conflicts(&items));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let classes: usize = items.iter().map(|item| item.classes.len()).sum();
    let plan = merge_plan(&grid, settings, classes)?;
    let mut durations_s = BTreeMap::new();
    durations_s.insert("validate", started.elapsed().as_secs_f64());

    monitor.stage("fingerprints");
    let started = Instant::now();
    let files: Vec<(&'static str, &Path)> = std::iter::once(("dem", job.dem.as_path()))
        .chain(job.stack.iter().map(|item| ("stack_item", item.path())))
        .collect();
    let fingerprints = fingerprints(&files, cancelled)?;
    durations_s.insert("fingerprints", started.elapsed().as_secs_f64());

    Ok(PreparedMerge {
        job,
        grid,
        items,
        warnings,
        plan,
        fingerprints,
        durations_s,
    })
}

/// Bytes held per class of the stack: item plan, class maps, per-class
/// counts and the class table (generous).
const BYTES_PER_CLASS: usize = 256;

/// Memory plan: one tiled grid of [`MergeCell`], raster read windows (and
/// their NoData-mapped row), output blocks with their one-cell halo, and
/// `classes` class entries. Vector features are read one at a time and
/// burnt one part at a time; a part's coordinates are not counted.
fn merge_plan(
    grid: &GridSpec,
    settings: &ExecutionSettings,
    classes: usize,
) -> Result<ResourcePlan> {
    let geometry = TileGeometry::new(grid.rows, grid.cols, accessmod_core::store::DEFAULT_TILE)?;
    let per_tile = TiledGrid::<MergeCell>::tile_bytes(geometry) as u64;
    let halo = (BLOCK + 2) * (BLOCK + 2);
    let buffer_bytes = (STRIP_ROWS * CHUNK_COLS * 8
        + CHUNK_COLS * std::mem::size_of::<Option<i64>>()
        + halo * (std::mem::size_of::<MergeCell>() + 2 + std::mem::size_of::<Option<i32>>())
        + BLOCK * BLOCK * (4 + 1)
        + classes * BYTES_PER_CLASS) as u64;
    ResourcePlan::for_tiles(grid.rows, grid.cols, settings, per_tile, buffer_bytes, 0)
}

/// Class a raster value or attribute takes in the merged land cover, or why
/// it cannot.
fn output_class(value: i64) -> std::result::Result<i32, String> {
    match i32::try_from(value) {
        Ok(class) if class != MERGED_NODATA => Ok(class),
        _ => Err(format!(
            "class {value} does not fit the merged land cover (32-bit, {MERGED_NODATA} is NoData)"
        )),
    }
}

fn plan_land_cover(
    item: &LandCoverItem,
    grid: &GridSpec,
    at: &str,
    position: usize,
    rank: &mut u32,
    warnings: &mut Vec<Warning>,
    cancelled: &dyn Fn() -> bool,
) -> Result<ItemPlan> {
    let (dataset, alignment) = open_class_raster(&item.path, grid, at)?;
    let band = AlignedBand::new(&dataset, &item.path, alignment, grid)?;
    let nodata = integer_nodata(band.nodata());
    let mut counts: BTreeMap<i64, u64> = BTreeMap::new();
    let mut too_many = false;
    band.for_each_covered_row::<i64>(cancelled, |_, _, values| {
        for value in values.iter().filter(|v| Some(**v) != nodata) {
            if let Some(count) = counts.get_mut(value) {
                *count += 1;
            } else if counts.len() < MAX_ITEM_CLASSES {
                counts.insert(*value, 1);
            } else {
                too_many = true;
            }
        }
        Ok(())
    })?;
    if too_many {
        return Err(Error::invalid(format!(
            "{at}: more than {MAX_ITEM_CLASSES} distinct classes; is it a class raster?"
        )));
    }
    let reclass: BTreeMap<i64, i64> = item.reclass.iter().map(|r| (r.from, r.to)).collect();
    let labels: BTreeMap<i64, &str> = item
        .labels
        .iter()
        .map(|l| (l.class, l.label.as_str()))
        .collect();
    for from in reclass.keys().filter(|c| !counts.contains_key(c)) {
        warnings.push(Warning {
            code: "reclass_class_absent",
            message: format!("{at}: reclass of class {from}, which does not occur in the grid"),
        });
    }
    let mut issues = Vec::new();
    let mut classes = Vec::new();
    for (&source, &count) in &counts {
        let value = reclass.get(&source).copied().unwrap_or(source);
        match output_class(value) {
            Ok(class) => classes.push(ItemClass {
                class,
                source_class: source,
                label: labels.get(&source).map(|l| (*l).to_owned()),
                rank: *rank,
                count,
            }),
            Err(why) => issues.push(format!("{at}: {why}")),
        }
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    if classes.is_empty() {
        warnings.push(Warning {
            code: "stack_item_empty",
            message: format!("{at}: no class inside the grid"),
        });
    }
    *rank += 1;
    Ok(ItemPlan {
        position,
        kind: "land_cover",
        path: item.path.clone(),
        layer: None,
        reprojected: None,
        classes,
        features: None,
        barrier_rank: None,
    })
}

fn plan_road(
    item: &RoadItem,
    grid: &GridSpec,
    at: &str,
    position: usize,
    rank: &mut u32,
    warnings: &mut Vec<Warning>,
    cancelled: &dyn Fn() -> bool,
) -> Result<ItemPlan> {
    let vector = VectorLayer::open(
        &item.path,
        item.layer.as_deref(),
        item.filter.as_deref(),
        grid,
        at,
    )?;
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let class_index = vector.field_index(&layer, &item.class_field)?;
    let label_index = match &item.label_field {
        Some(name) => Some(vector.field_index(&layer, name)?),
        None => None,
    };
    // Class → (features, first non-empty label).
    let mut found: BTreeMap<i64, (u64, Option<String>)> = BTreeMap::new();
    let mut without_class = 0u64;
    for (index, feature) in vector.features(&mut layer)?.enumerate() {
        if index % 4096 == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        let value = feature
            .field(class_index)
            .map_err(Error::gdal(&item.path))?;
        let Some(class) = value.and_then(integer_attribute) else {
            without_class += 1;
            continue;
        };
        if found.len() >= MAX_ITEM_CLASSES && !found.contains_key(&class) {
            return Err(vector.invalid(format!(
                "more than {MAX_ITEM_CLASSES} distinct classes in {:?}; is it a class field?",
                item.class_field
            )));
        }
        let label = match label_index {
            Some(index) => feature
                .field_as_string(index)
                .map_err(Error::gdal(&item.path))?
                .filter(|l| !l.trim().is_empty()),
            None => None,
        };
        let entry = found.entry(class).or_insert((0, None));
        entry.0 += 1;
        if entry.1.is_none() {
            entry.1 = label;
        }
    }
    if without_class > 0 {
        warnings.push(Warning {
            code: "road_without_class",
            message: format!(
                "{at}: {without_class} features have no integer value in {:?} and are left out",
                item.class_field
            ),
        });
    }
    let selected: Vec<i64> = match &item.classes {
        Some(list) => {
            for class in list.iter().filter(|c| !found.contains_key(c)) {
                warnings.push(Warning {
                    code: "road_class_absent",
                    message: format!("{at}: class {class} has no feature"),
                });
            }
            let ignored: Vec<String> = found
                .keys()
                .filter(|c| !list.contains(c))
                .map(i64::to_string)
                .collect();
            if !ignored.is_empty() {
                warnings.push(Warning {
                    code: "road_class_ignored",
                    message: format!(
                        "{at}: classes not listed in classes are left out: {}",
                        ignored.join(", ")
                    ),
                });
            }
            list.clone()
        }
        None => found.keys().copied().collect(),
    };
    let mut issues = Vec::new();
    let mut classes = Vec::new();
    for source_class in selected {
        let value = if item.add_1000 && source_class < ROAD_CLASS_OFFSET {
            source_class + ROAD_CLASS_OFFSET
        } else {
            source_class
        };
        let (count, label) = found.get(&source_class).cloned().unwrap_or_default();
        match output_class(value) {
            Ok(class) => classes.push(ItemClass {
                class,
                source_class,
                label,
                rank: *rank,
                count,
            }),
            Err(why) => issues.push(format!("{at}: {why}")),
        }
        *rank += 1;
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    reprojection_warning(&vector, at, warnings);
    Ok(ItemPlan {
        position,
        kind: "road",
        path: item.path.clone(),
        reprojected: Some(vector.reprojected()),
        layer: Some(vector.name),
        classes,
        features: None,
        barrier_rank: None,
    })
}

fn plan_barrier(
    item: &BarrierItem,
    grid: &GridSpec,
    at: &str,
    position: usize,
    warnings: &mut Vec<Warning>,
) -> Result<ItemPlan> {
    let vector = VectorLayer::open(
        &item.path,
        item.layer.as_deref(),
        item.filter.as_deref(),
        grid,
        at,
    )?;
    reprojection_warning(&vector, at, warnings);
    Ok(ItemPlan {
        position,
        kind: "barrier",
        path: item.path.clone(),
        reprojected: Some(vector.reprojected()),
        layer: Some(vector.name),
        classes: Vec::new(),
        features: None,
        barrier_rank: None,
    })
}

fn reprojection_warning(vector: &VectorLayer, at: &str, warnings: &mut Vec<Warning>) {
    if vector.reprojected() {
        warnings.push(Warning {
            code: "stack_item_reprojected",
            message: format!(
                "{at}: geometries of layer {:?} are reprojected to the DEM CRS",
                vector.name
            ),
        });
    }
}

/// Every class produced by more than one item: AccessMod 5 refuses to merge
/// such a stack, since the merged class could not be traced to one item.
fn class_conflicts(items: &[ItemPlan]) -> Vec<String> {
    let mut producers: BTreeMap<i32, BTreeSet<usize>> = BTreeMap::new();
    for item in items {
        for class in &item.classes {
            producers
                .entry(class.class)
                .or_default()
                .insert(item.position);
        }
    }
    let conflicts: Vec<String> = producers
        .iter()
        .filter(|(_, positions)| positions.len() > 1)
        .map(|(class, positions)| {
            let listed: Vec<String> = positions
                .iter()
                .map(|p| format!("{} ({})", p, items[p - 1].kind))
                .collect();
            format!(
                "class {class} comes from stack items {}",
                listed.join(" and ")
            )
        })
        .collect();
    if conflicts.is_empty() {
        return conflicts;
    }
    let mut issues = conflicts;
    issues.push(
        "each class must come from one item: use add_1000 or classes on roads, or reclass \
         on land_cover items"
            .into(),
    );
    issues
}

/// Merge result counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MergeSummary {
    /// Cells of the grid.
    pub cells: u64,
    /// Cells with a class in the merged land cover.
    pub classified_cells: u64,
    /// Cells emptied by a barrier.
    pub barrier_cells: u64,
    /// Cells no item covers.
    pub uncovered_cells: u64,
    /// Cells of the bridge map (road cells matching the bridge pattern,
    /// including those already empty), when bridges are cleaned.
    pub bridge_cells: u64,
    /// Classified cells emptied as bridges.
    pub bridges_removed: u64,
    /// Classes of the merged land cover.
    pub classes: Vec<MergedClass>,
}

/// One class of the merged land cover (`classes.csv`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergedClass {
    /// Class value.
    pub class: i32,
    /// Label, if known.
    pub label: Option<String>,
    /// Stack item it comes from (1-based position).
    pub stack_item: usize,
    /// Cells.
    pub cells: u64,
}

/// `run.json` of a merge.
pub type MergeReport = Report<MergeRecord, MergeStore>;

/// Deterministic part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct MergeRecord {
    /// Job with resolved paths and defaults.
    pub job: MergeLandCoverJob,
    /// Input files.
    pub inputs: Vec<Fingerprint>,
    /// DEM grid.
    pub grid: GridReport,
    /// Stack items with their classes and feature counts.
    pub stack: Vec<ItemPlan>,
    /// Result counters.
    pub result: MergeSummary,
    /// Non-blocking problems.
    pub warnings: Vec<Warning>,
    /// Differences from AccessMod 5 that apply.
    pub deviations: Vec<Deviation>,
}

/// Execution details of a merge.
#[derive(Debug, Clone, Serialize)]
pub struct MergeStore {
    /// Cache counters of the merge grid.
    pub store: StoreStats,
}

/// Deviations from AccessMod 5 that apply to every merge.
pub fn deviations() -> Vec<Deviation> {
    vec![
        Deviation {
            code: "line_rasterisation",
            message: "Lines burn every cell they touch, computed from the vertices on the \
                      DEM lattice. AccessMod 5 uses v.to.rast -d; both differ on a few \
                      cells where a line passes close to a cell corner.",
        },
        Deviation {
            code: "stack_rules",
            message: "Any number of land_cover items is accepted and no output tag is \
                      needed. AccessMod 5 requires exactly one stack land cover and a tag.",
        },
        Deviation {
            code: "barrier_position",
            message: "Every barrier empties only the items below it in the stack. AccessMod 5 \
                      applies the first barrier to every item, wherever it is; the two agree \
                      when the first barrier is at the top.",
        },
        Deviation {
            code: "labels_verbatim",
            message: "Labels are kept as given. AccessMod 5 lowercases them and replaces \
                      punctuation and spaces with underscores.",
        },
    ]
}

/// Merge a prepared stack and write every output atomically.
pub fn run(
    mut prepared: PreparedMerge,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<(PathBuf, MergeReport)> {
    let cancelled = monitor.cancelled();
    let staging = Staging::create(&prepared.job.output_dir)?;
    let grid = &prepared.grid;
    let geometry = TileGeometry::new(grid.rows, grid.cols, prepared.plan.tile)?;
    let mut cells = TiledGrid::new(
        geometry,
        MergeCell::EMPTY,
        prepared.plan.resident_tiles,
        &settings.tmp_dir,
        "merge",
    )?;

    monitor.stage("paint");
    let started = Instant::now();
    for (item, plan) in prepared.job.stack.iter().zip(prepared.items.iter_mut()) {
        let at = format!("stack item {} ({})", plan.position, plan.kind);
        match item {
            StackItem::LandCover(item) => {
                paint_land_cover(item, plan, &at, grid, &mut cells, cancelled)?
            }
            StackItem::Road(item) => {
                plan.features = Some(paint_road(item, plan, &at, grid, &mut cells, cancelled)?);
            }
            StackItem::Barrier(item) => {
                let vector = open_item_layer(&item.path, &item.layer, &item.filter, grid, &at)?;
                let rank = plan
                    .barrier_rank
                    .ok_or_else(|| Error::invalid(format!("{at}: barrier without a rank")))?;
                let barrier = move |cell: &mut MergeCell| cell.bar(rank);
                plan.features = Some(burn_layer(
                    &vector,
                    grid,
                    &mut cells,
                    Burn::LinesAndPolygons,
                    cancelled,
                    |_| Ok(Some(barrier)),
                )?);
            }
        }
    }
    prepared
        .durations_s
        .insert("paint", started.elapsed().as_secs_f64());

    monitor.stage("write_outputs");
    let started = Instant::now();
    let mut result = write_outputs(
        staging.path(),
        grid,
        &mut cells,
        prepared.job.clean_bridges,
        settings.threads,
        cancelled,
    )?;
    // Labels and items of the classes present in the output.
    let mut origin: BTreeMap<i32, (usize, Option<String>)> = BTreeMap::new();
    for item in &prepared.items {
        for class in &item.classes {
            let entry = origin.entry(class.class).or_insert((item.position, None));
            if entry.1.is_none() {
                entry.1.clone_from(&class.label);
            }
        }
    }
    for class in &mut result.classes {
        if let Some((position, label)) = origin.get(&class.class) {
            class.stack_item = *position;
            class.label.clone_from(label);
        }
    }
    let mut csv = CsvWriter::create(
        &staging.path().join(CLASSES_FILE),
        "class,label,stack_item,cells",
    )?;
    for class in &result.classes {
        let label = class
            .label
            .as_ref()
            .map(|l| csv_field(&serde_json::Value::String(l.clone())))
            .unwrap_or_default();
        csv.record(&[
            class.class.to_string(),
            label,
            class.stack_item.to_string(),
            class.cells.to_string(),
        ])?;
    }
    csv.finish()?;
    prepared
        .durations_s
        .insert("write_outputs", started.elapsed().as_secs_f64());

    let mut warnings = prepared.warnings.clone();
    for item in &prepared.items {
        let Some(features) = &item.features else {
            continue;
        };
        if features.unsupported_geometry > 0 {
            warnings.push(Warning {
                code: "unsupported_geometry",
                message: format!(
                    "stack item {} ({}): {} features have no line{} geometry and are left out",
                    item.position,
                    item.kind,
                    features.unsupported_geometry,
                    if item.kind == "barrier" {
                        " or polygon"
                    } else {
                        ""
                    }
                ),
            });
        }
        if features.outside_grid > 0 {
            warnings.push(Warning {
                code: "features_outside_grid",
                message: format!(
                    "stack item {} ({}): {} features burn no cell (outside the grid, or \
                     polygons holding no cell centre)",
                    item.position, item.kind, features.outside_grid
                ),
            });
        }
    }
    let store = cells.stats();
    let PreparedMerge {
        job,
        grid,
        items,
        plan,
        fingerprints,
        durations_s,
        ..
    } = prepared;
    let report = Report::new(
        MergeRecord {
            job,
            inputs: fingerprints,
            grid: GridReport::of(&grid),
            stack: items,
            result,
            warnings,
            deviations: deviations(),
        },
        Execution::now(settings, plan, durations_s, MergeStore { store }),
    );
    let output_dir = output::finish(staging, &report.run.job, &report, cancelled)?;
    Ok((output_dir, report))
}

/// The layer of a vector stack item, checked.
fn open_item_layer(
    path: &Path,
    layer: &Option<String>,
    filter: &Option<String>,
    grid: &GridSpec,
    at: &str,
) -> Result<VectorLayer> {
    VectorLayer::open(path, layer.as_deref(), filter.as_deref(), grid, at)
}

/// Item classes keyed by their value in the item: (class, rank).
fn class_ranks(plan: &ItemPlan) -> BTreeMap<i64, (i32, u32)> {
    plan.classes
        .iter()
        .map(|c| (c.source_class, (c.class, c.rank)))
        .collect()
}

fn paint_land_cover(
    item: &LandCoverItem,
    plan: &ItemPlan,
    at: &str,
    grid: &GridSpec,
    cells: &mut TiledGrid<MergeCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let (dataset, alignment) = open_class_raster(&item.path, grid, at)?;
    let band = AlignedBand::new(&dataset, &item.path, alignment, grid)?;
    let nodata = integer_nodata(band.nodata());
    let classes = class_ranks(plan);
    band.for_each_covered_row::<i64>(cancelled, |row, col, values| {
        for (k, value) in values.iter().enumerate() {
            if Some(*value) == nodata {
                continue;
            }
            let Some(&(class, rank)) = classes.get(value) else {
                continue;
            };
            let mut cell = cells.get(row, col + k)?;
            cell.paint(class, rank);
            cells.set(row, col + k, cell)?;
        }
        Ok(())
    })
}

fn paint_road(
    item: &RoadItem,
    plan: &ItemPlan,
    at: &str,
    grid: &GridSpec,
    cells: &mut TiledGrid<MergeCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<FeatureCounts> {
    let vector = open_item_layer(&item.path, &item.layer, &item.filter, grid, at)?;
    let class_index = {
        let dataset = vector.dataset()?;
        let layer = vector.layer(&dataset)?;
        vector.field_index(&layer, &item.class_field)?
    };
    let classes = class_ranks(plan);
    // Roads are lines: AccessMod 5 rasterises them with `v.to.rast
    // type=line`.
    burn_layer(&vector, grid, cells, Burn::Lines, cancelled, |feature| {
        let value = feature
            .field(class_index)
            .map_err(Error::gdal(&item.path))?;
        Ok(value
            .and_then(integer_attribute)
            .and_then(|v| classes.get(&v))
            .map(|&(class, rank)| {
                move |cell: &mut MergeCell| {
                    cell.paint(class, rank);
                    cell.flags |= MergeCell::ROAD;
                }
            }))
    })
}

/// Stream the merged land cover (and the bridge map) block by block, each
/// block read with a one-cell halo for the bridge test.
fn write_outputs(
    dir: &Path,
    grid: &GridSpec,
    cells: &mut TiledGrid<MergeCell>,
    clean_bridges: bool,
    threads: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<MergeSummary> {
    let (rows, cols) = (grid.rows, grid.cols);
    let merged_tif = GeoTiff::create::<i32>(
        &dir.join(MERGED_FILE),
        grid,
        2,
        threads,
        f64::from(MERGED_NODATA),
    )?;
    let bridges_tif = if clean_bridges {
        Some(GeoTiff::create::<u8>(
            &dir.join(BRIDGES_FILE),
            grid,
            2,
            threads,
            f64::from(BRIDGE_NODATA),
        )?)
    } else {
        None
    };

    let mut summary = MergeSummary {
        cells: (rows * cols) as u64,
        ..MergeSummary::default()
    };
    let mut class_cells: BTreeMap<i32, u64> = BTreeMap::new();
    let mut window = vec![MergeCell::EMPTY; BLOCK + 2];
    // Rows of the block with its halo.
    fn halo_rows<T>() -> Vec<Vec<T>> {
        (0..BLOCK + 2)
            .map(|_| Vec::with_capacity(BLOCK + 2))
            .collect()
    }
    let mut filled: Vec<Vec<bool>> = halo_rows();
    let mut road: Vec<Vec<bool>> = halo_rows();
    let mut values: Vec<Vec<Option<i32>>> = halo_rows();
    let mut bridge = vec![false; BLOCK + 2];
    let mut merged_block = Vec::with_capacity(BLOCK * BLOCK);
    let mut bridge_block = Vec::with_capacity(BLOCK * BLOCK);
    for_each_window(
        0..rows,
        0..cols,
        (BLOCK, BLOCK),
        cancelled,
        |block_rows, block_cols| {
            let (row0, col0, width) = (block_rows.start, block_cols.start, block_cols.len());
            let top = row0.saturating_sub(1);
            let bottom = (block_rows.end + 1).min(rows);
            let left = col0.saturating_sub(1);
            let right = (block_cols.end + 1).min(cols);
            let span = right - left;
            for (k, row) in (top..bottom).enumerate() {
                cells.read_row(row, left, &mut window[..span])?;
                filled[k].clear();
                road[k].clear();
                values[k].clear();
                for cell in &window[..span] {
                    let value = cell.value();
                    values[k].push(value);
                    filled[k].push(value.is_some());
                    road[k].push(cell.is_road());
                }
            }
            for row in block_rows.clone() {
                let k = row - top;
                let inner = col0 - left;
                if clean_bridges {
                    let above = (row > top).then(|| filled[k - 1].as_slice());
                    let below = (row + 1 < bottom).then(|| filled[k + 1].as_slice());
                    bridge_row(above, &filled[k], below, &road[k], &mut bridge[..span]);
                }
                for c in inner..inner + width {
                    let value = values[k][c];
                    let is_bridge = clean_bridges && bridge[c];
                    if is_bridge {
                        summary.bridge_cells += 1;
                        if value.is_some() {
                            summary.bridges_removed += 1;
                        }
                    }
                    let kept = if is_bridge { None } else { value };
                    if let Some(class) = kept {
                        summary.classified_cells += 1;
                        *class_cells.entry(class).or_default() += 1;
                    }
                    merged_block.push(kept.unwrap_or(MERGED_NODATA));
                    if clean_bridges {
                        bridge_block.push(if is_bridge { 1 } else { BRIDGE_NODATA });
                    }
                }
            }
            merged_tif.write(block_rows.clone(), block_cols.clone(), &mut merged_block)?;
            if let Some(tif) = &bridges_tif {
                tif.write(block_rows, block_cols, &mut bridge_block)?;
            }
            Ok(())
        },
    )?;
    // Barrier and uncovered cells.
    cells.for_each_row_piece(BLOCK, |_, col, piece| {
        if col == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        for cell in piece {
            if cell.is_barrier() {
                summary.barrier_cells += 1;
            } else if cell.rank == MergeCell::NO_RANK {
                summary.uncovered_cells += 1;
            }
        }
        Ok(())
    })?;
    merged_tif.close()?;
    if let Some(tif) = bridges_tif {
        tif.close()?;
    }
    summary.classes = class_cells
        .into_iter()
        .map(|(class, cells)| MergedClass {
            class,
            label: None,
            stack_item: 0,
            cells,
        })
        .collect();
    Ok(summary)
}

/// Merge land cover.
#[derive(Debug)]
pub struct MergeLandCover;

impl Analysis for MergeLandCover {
    const NAME: &'static str = "merge-land-cover";
    const OUTPUTS: &'static [Output] = &[
        Output {
            file: MERGED_FILE,
            class: Some(DataClass::LandCoverMerged),
        },
        Output {
            file: CLASSES_FILE,
            class: Some(DataClass::LandCoverTable),
        },
        Output {
            file: BRIDGES_FILE,
            class: None,
        },
    ];
    type Job = MergeLandCoverJob;
    type Prepared = PreparedMerge;
    type Report = MergeReport;

    fn prepare(
        job: MergeLandCoverJob,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<PreparedMerge> {
        prepare(job, settings, monitor)
    }

    fn run(
        prepared: PreparedMerge,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<(PathBuf, MergeReport)> {
        run(prepared, settings, monitor)
    }

    fn warnings(prepared: &PreparedMerge) -> &[Warning] {
        &prepared.warnings
    }

    fn describe(prepared: &PreparedMerge) -> String {
        let classes: usize = prepared.items.iter().map(|i| i.classes.len()).sum();
        format!(
            "merge: {} stack items, {} classes on a {} x {} grid",
            prepared.items.len(),
            classes,
            prepared.grid.cols,
            prepared.grid.rows
        )
    }

    fn validation(prepared: &PreparedMerge) -> serde_json::Value {
        serde_json::json!({
            "job": prepared.job,
            "stack": prepared.items,
            "warnings": prepared.warnings,
            "plan": prepared.plan,
        })
    }

    fn outcome(report: &MergeReport) -> String {
        let r = &report.run.result;
        format!(
            "{} classified cells in {} classes, {} barrier cells, {} bridges removed",
            r.classified_cells,
            r.classes.len(),
            r.barrier_cells,
            r.bridges_removed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "schema_version": 1,
        "dem": "dem.tif",
        "stack": [
            {"type": "road", "path": "roads.gpkg", "class_field": "class"},
            {"type": "land_cover", "path": "lc.tif",
             "labels": [{"class": 1, "label": "forest"}]},
            {"type": "barrier", "path": "rivers.gpkg"}
        ],
        "output_dir": "out"
    }"#;

    #[test]
    fn minimal_job_parses_with_defaults() {
        let job = MergeLandCoverJob::from_json(MINIMAL).expect("valid job");
        assert!(job.check().is_empty(), "{:?}", job.check());
        assert!(job.clean_bridges);
        let StackItem::Road(road) = &job.stack[0] else {
            panic!("first item is a road");
        };
        assert!(road.add_1000);
        assert_eq!(road.classes, None);
        let job = job.resolve_paths(Path::new("/data"));
        assert_eq!(job.stack[2].path(), Path::new("/data/rivers.gpkg"));
    }

    #[test]
    fn unknown_fields_and_types_are_rejected() {
        let text = MINIMAL.replace("\"class_field\"", "\"clas_field\": 1, \"class_field\"");
        assert!(MergeLandCoverJob::from_json(&text).is_err());
        let text = MINIMAL.replace("\"barrier\"", "\"wall\"");
        assert!(MergeLandCoverJob::from_json(&text).is_err());
    }

    #[test]
    fn semantic_problems_are_all_reported() {
        let mut job = MergeLandCoverJob::from_json(MINIMAL).expect("valid job");
        job.schema_version = 2;
        if let StackItem::Road(road) = &mut job.stack[0] {
            road.class_field = " ".into();
            road.classes = Some(vec![1, 2, 1]);
        }
        if let StackItem::LandCover(lc) = &mut job.stack[1] {
            lc.reclass = vec![Reclass { from: 3, to: 4 }, Reclass { from: 3, to: 5 }];
        }
        assert_eq!(job.check().len(), 4, "{:#?}", job.check());
        job.stack.truncate(0);
        assert!(job.check().iter().any(|i| i.contains("no land_cover")));
    }

    #[test]
    fn conflicts_name_the_items() {
        let class = |class, rank| ItemClass {
            class,
            source_class: i64::from(class),
            label: None,
            rank,
            count: 1,
        };
        let item = |position, kind, classes| ItemPlan {
            position,
            kind,
            path: PathBuf::from("x"),
            layer: None,
            reprojected: None,
            classes,
            features: None,
            barrier_rank: None,
        };
        let items = vec![
            item(1, "road", vec![class(1001, 0), class(2, 1)]),
            item(2, "land_cover", vec![class(1, 2), class(2, 2)]),
        ];
        let issues = class_conflicts(&items);
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues[0].contains("class 2 comes from stack items 1 (road) and 2 (land_cover)"));
    }

    #[test]
    fn output_classes_fit_32_bits() {
        assert!(output_class(i64::from(i32::MIN)).is_err());
        assert!(output_class(1 << 40).is_err());
    }
}
