//! Merge land cover (schema version 1): a stack of land-cover rasters, road
//! layers and barrier layers merged on the DEM lattice, as the AccessMod 5
//! toolbox does (`modules/amAnalysisMergeLandCover`).
//!
//! Validation (`prepare`) reads every item once to list its classes and
//! refuses class conflicts; execution (`run`) paints the items into a tiled
//! grid by priority (`accessmod_core::merge`) and streams the outputs.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accessmod_core::merge::{MergeCell, bridge_row, line_spans, polygon_spans};
use accessmod_core::store::{StoreStats, TileGeometry, TiledGrid};
use gdal::raster::{Buffer, RasterCreationOptions};
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use gdal::vector::{
    FieldValue, Geometry, Layer, LayerAccess, OGRwkbGeometryType, geometry_type_flatten,
};
use gdal::{Dataset, DatasetOptions, DriverManager, GdalOpenFlags};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::grid::{Alignment, GridSpec};
use crate::inputs::{CHUNK_COLS, STRIP_ROWS, integer_nodata, open_class_raster};
use crate::job::SCHEMA_VERSION;
use crate::output::{self, Staging};
use crate::referral::csv_field;
use crate::resources;
use crate::travel_time::{
    Deviation, ExecutionSettings, Fingerprint, GridReport, RUN_REPORT_VERSION, ResourcePlan,
    Warning, sha256_file, versions,
};

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
/// GeoTIFF block side of the outputs, cells.
const BLOCK: usize = 256;

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
    pub dem: PathBuf,
    /// Stack items. A cell takes the class of the first non-barrier item
    /// covering it; a barrier anywhere in the stack empties the cell.
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
    /// Barrier lines or polygons (stack barrier).
    Barrier(BarrierItem),
}

/// Land-cover raster item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LandCoverItem {
    /// Raster with integer classes, on the DEM lattice and in its CRS.
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

    fn kind(&self) -> &'static str {
        match self {
            Self::LandCover(_) => "land_cover",
            Self::Road(_) => "road",
            Self::Barrier(_) => "barrier",
        }
    }
}

impl MergeLandCoverJob {
    /// Parse a job from JSON.
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|e| Error::invalid(format!("job JSON: {e}")))
    }

    /// Read and parse a job file; relative paths are resolved against the
    /// file's directory.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(Error::file(path))?;
        let job = Self::from_json(&text)?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Ok(job.resolve_paths(base))
    }

    /// JSON schema of the job document.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(MergeLandCoverJob))
            .expect("a generated schema serialises to JSON")
    }

    /// Paths made absolute against `base` (when relative).
    pub fn resolve_paths(mut self, base: &Path) -> Self {
        let absolute = |p: &mut PathBuf| {
            if !p.is_absolute() {
                *p = base.join(&*p);
            }
        };
        absolute(&mut self.dem);
        absolute(&mut self.output_dir);
        for item in &mut self.stack {
            match item {
                StackItem::LandCover(item) => absolute(&mut item.path),
                StackItem::Road(item) => absolute(&mut item.path),
                StackItem::Barrier(item) => absolute(&mut item.path),
            }
        }
        self
    }

    /// Problems that make the job invalid regardless of its input data.
    pub fn check(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.schema_version != SCHEMA_VERSION {
            issues.push(format!(
                "schema_version is {}, this engine reads version {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
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
    let mut seen = BTreeSet::new();
    let mut repeated = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            repeated.insert(value);
        }
    }
    for value in repeated {
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

/// Feature counts of a vector item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FeatureCounts {
    /// Features read (after the attribute filter).
    pub read: u64,
    /// Features burnt into the grid.
    pub used: u64,
    /// Features without a usable class (road) or not in `classes`.
    pub without_class: u64,
    /// Features without geometry, or with points or curves only.
    pub unsupported_geometry: u64,
    /// Line parts burnt.
    pub lines: u64,
    /// Polygon parts burnt.
    pub polygons: u64,
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
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedMerge> {
    let started = Instant::now();
    let mut issues = job.check();
    if let Err(Error::Validation { issues: more }) = output::check_output_dir(&job.output_dir) {
        issues.extend(more);
    }
    if !job.dem.exists() {
        issues.push(format!("dem: {} does not exist", job.dem.display()));
    }
    for (i, item) in job.stack.iter().enumerate() {
        if !item.path().exists() {
            issues.push(format!(
                "stack item {} ({}): {} does not exist",
                i + 1,
                item.kind(),
                item.path().display()
            ));
        }
    }
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }

    let dem = Dataset::open(&job.dem).map_err(Error::gdal(&job.dem))?;
    let grid = GridSpec::from_dem(&dem, &job.dem).map_err(|issues| Error::Validation { issues })?;
    drop(dem);
    let plan = merge_plan(&grid, settings)?;
    gdal::config::set_config_option("GDAL_CACHEMAX", &plan.gdal_cache_bytes.to_string())
        .map_err(Error::gdal(&job.dem))?;

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
            StackItem::Road(item) => {
                plan_road(item, &grid, &at, position, &mut rank, &mut warnings)
            }
            StackItem::Barrier(item) => plan_barrier(item, &grid, &at, position, &mut warnings),
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
    let mut durations_s = BTreeMap::new();
    durations_s.insert("validate", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let mut fingerprints = vec![sha256_file("dem", &job.dem)?];
    for item in &job.stack {
        fingerprints.push(sha256_file("stack_item", item.path())?);
    }
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

/// Memory plan: one tiled grid of [`MergeCell`], raster read windows and
/// output blocks with their one-cell halo.
fn merge_plan(grid: &GridSpec, settings: &ExecutionSettings) -> Result<ResourcePlan> {
    let geometry = TileGeometry::new(grid.rows, grid.cols, accessmod_core::store::DEFAULT_TILE)?;
    let per_tile = TiledGrid::<MergeCell>::tile_bytes(geometry) as u64;
    let halo = (BLOCK + 2) * (BLOCK + 2);
    let buffer_bytes = (STRIP_ROWS * CHUNK_COLS * 8
        + halo * (std::mem::size_of::<MergeCell>() + 2)
        + BLOCK * BLOCK * (4 + 1)) as u64;
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
    let mut counts: BTreeMap<i64, u64> = BTreeMap::new();
    let mut too_many = false;
    for_each_raster_window(
        &dataset,
        &item.path,
        &alignment,
        grid,
        cancelled,
        |_, _, values| {
            for value in values.iter().flatten() {
                if let Some(count) = counts.get_mut(value) {
                    *count += 1;
                } else if counts.len() < MAX_ITEM_CLASSES {
                    counts.insert(*value, 1);
                } else {
                    too_many = true;
                }
            }
            Ok(())
        },
    )?;
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
    })
}

fn plan_road(
    item: &RoadItem,
    grid: &GridSpec,
    at: &str,
    position: usize,
    rank: &mut u32,
    warnings: &mut Vec<Warning>,
) -> Result<ItemPlan> {
    let source = VectorSource::open(
        &item.path,
        item.layer.as_deref(),
        item.filter.as_deref(),
        grid,
        at,
    )?;
    let dataset = open_vector(&item.path)?;
    let mut layer = source.layer(&dataset)?;
    let invalid = |issue: String| Error::invalid(format!("{at}: {issue}"));
    let defn_index = |name: &str| layer.defn().field_index(name);
    let class_index = defn_index(&item.class_field).map_err(|_| {
        invalid(format!(
            "layer {:?} has no field {:?}",
            source.layer_name, item.class_field
        ))
    })?;
    let label_index = match &item.label_field {
        Some(name) => Some(defn_index(name).map_err(|_| {
            invalid(format!(
                "layer {:?} has no field {name:?}",
                source.layer_name
            ))
        })?),
        None => None,
    };
    // Class → (features, first non-empty label).
    let mut found: BTreeMap<i64, (u64, Option<String>)> = BTreeMap::new();
    let mut without_class = 0u64;
    for feature in layer.features() {
        let value = feature
            .field(class_index)
            .map_err(Error::gdal(&item.path))?;
        let Some(class) = value.and_then(integer_value) else {
            without_class += 1;
            continue;
        };
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
    reprojection_warning(&source.layer_name, source.transform.is_some(), at, warnings);
    Ok(ItemPlan {
        position,
        kind: "road",
        path: item.path.clone(),
        layer: Some(source.layer_name),
        reprojected: Some(source.transform.is_some()),
        classes,
        features: None,
    })
}

fn plan_barrier(
    item: &BarrierItem,
    grid: &GridSpec,
    at: &str,
    position: usize,
    warnings: &mut Vec<Warning>,
) -> Result<ItemPlan> {
    let source = VectorSource::open(
        &item.path,
        item.layer.as_deref(),
        item.filter.as_deref(),
        grid,
        at,
    )?;
    reprojection_warning(&source.layer_name, source.transform.is_some(), at, warnings);
    Ok(ItemPlan {
        position,
        kind: "barrier",
        path: item.path.clone(),
        layer: Some(source.layer_name),
        reprojected: Some(source.transform.is_some()),
        classes: Vec::new(),
        features: None,
    })
}

fn reprojection_warning(layer: &str, reprojected: bool, at: &str, warnings: &mut Vec<Warning>) {
    if reprojected {
        warnings.push(Warning {
            code: "stack_item_reprojected",
            message: format!("{at}: geometries of layer {layer:?} are reprojected to the DEM CRS"),
        });
    }
}

/// Integer class of an attribute: integers, whole reals and integer text.
fn integer_value(value: FieldValue) -> Option<i64> {
    match value {
        FieldValue::IntegerValue(v) => Some(i64::from(v)),
        FieldValue::Integer64Value(v) => Some(v),
        FieldValue::RealValue(v) if v.fract() == 0.0 && v.abs() < 9.0e15 => Some(v as i64),
        FieldValue::StringValue(text) => text.trim().parse().ok(),
        _ => None,
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

/// A vector layer of a stack item, checked, with its transform to the grid
/// CRS.
struct VectorSource {
    layer_name: String,
    filter: Option<String>,
    path: PathBuf,
    transform: Option<CoordTransform>,
}

fn open_vector(path: &Path) -> Result<Dataset> {
    let options = DatasetOptions {
        open_flags: GdalOpenFlags::GDAL_OF_VECTOR | GdalOpenFlags::GDAL_OF_READONLY,
        ..DatasetOptions::default()
    };
    Dataset::open_ex(path, options).map_err(Error::gdal(path))
}

impl VectorSource {
    /// Check the layer, its CRS and the attribute filter.
    fn open(
        path: &Path,
        layer: Option<&str>,
        filter: Option<&str>,
        grid: &GridSpec,
        at: &str,
    ) -> Result<Self> {
        let dataset = open_vector(path)?;
        let invalid = |issue: String| Error::invalid(format!("{at} {}: {issue}", path.display()));
        let mut found = match layer {
            Some(name) => dataset
                .layer_by_name(name)
                .map_err(|_| invalid(format!("no layer named {name:?}")))?,
            None => dataset
                .layer(0)
                .map_err(|_| invalid("has no layer".into()))?,
        };
        let layer_name = found.name();
        let layer_srs = found
            .spatial_ref()
            .ok_or_else(|| invalid(format!("layer {layer_name:?} has no CRS")))?;
        let transform = if layer_srs == grid.srs {
            None
        } else {
            let from = traditional(layer_srs);
            let to = traditional(grid.srs.clone());
            Some(CoordTransform::new(&from, &to).map_err(Error::gdal(path))?)
        };
        if let Some(filter) = filter {
            found
                .set_attribute_filter(filter)
                .map_err(|e| invalid(format!("invalid filter {filter:?} ({e})")))?;
            // Some drivers (GeoPackage) accept any filter and fail only when
            // reading; a query with the same dialect fails up front.
            let query = format!(
                "SELECT * FROM \"{}\" WHERE {filter}",
                layer_name.replace('"', "\"\"")
            );
            dataset
                .execute_sql(&query, None, gdal::vector::sql::Dialect::DEFAULT)
                .map_err(|e| invalid(format!("invalid filter {filter:?} ({e})")))?;
        }
        Ok(Self {
            layer_name,
            filter: filter.map(str::to_owned),
            path: path.to_path_buf(),
            transform,
        })
    }

    /// The layer of `dataset` (opened with [`open_vector`]), with the
    /// attribute filter set.
    fn layer<'a>(&self, dataset: &'a Dataset) -> Result<Layer<'a>> {
        let mut layer = dataset
            .layer_by_name(&self.layer_name)
            .map_err(Error::gdal(&self.path))?;
        if let Some(filter) = &self.filter {
            layer
                .set_attribute_filter(filter)
                .map_err(Error::gdal(&self.path))?;
        }
        Ok(layer)
    }
}

fn traditional(mut srs: SpatialRef) -> SpatialRef {
    srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    srs
}

/// Call `visit(row, first_col, values)` for every row piece of the raster
/// inside the grid, in strips of [`STRIP_ROWS`] rows and chunks of
/// [`CHUNK_COLS`] columns; `None` is NoData.
fn for_each_raster_window(
    dataset: &Dataset,
    path: &Path,
    alignment: &Alignment,
    grid: &GridSpec,
    cancelled: &dyn Fn() -> bool,
    mut visit: impl FnMut(usize, usize, &[Option<i64>]) -> Result<()>,
) -> Result<()> {
    let band = dataset.rasterband(1).map_err(Error::gdal(path))?;
    let nodata = integer_nodata(band.no_data_value());
    let row_first = alignment.row_offset.max(0) as usize;
    let row_end = (alignment.row_offset + alignment.rows as i64).min(grid.rows as i64) as usize;
    let Some((raster_col, col_first, width_all)) = alignment.row_window(row_first, grid.cols)
    else {
        return Ok(());
    };
    let mut values = Vec::with_capacity(CHUNK_COLS);
    for start in (row_first..row_end).step_by(STRIP_ROWS) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let height = STRIP_ROWS.min(row_end - start);
        for offset in (0..width_all).step_by(CHUNK_COLS) {
            let width = CHUNK_COLS.min(width_all - offset);
            let buffer = band
                .read_as::<i64>(
                    (
                        (raster_col + offset) as isize,
                        alignment.raster_row(start) as isize,
                    ),
                    (width, height),
                    (width, height),
                    None,
                )
                .map_err(Error::gdal(path))?;
            for r in 0..height {
                values.clear();
                values.extend(
                    buffer.data()[r * width..(r + 1) * width]
                        .iter()
                        .map(|&v| (Some(v) != nodata).then_some(v)),
                );
                visit(start + r, col_first + offset, &values)?;
            }
        }
    }
    Ok(())
}

/// Parts of a geometry in pixel coordinates of the grid.
#[derive(Debug, Default)]
struct Parts {
    lines: Vec<Vec<(f64, f64)>>,
    polygons: Vec<Vec<Vec<(f64, f64)>>>,
    unsupported: bool,
}

/// Map coordinates `points` to grid pixel coordinates, reprojecting first
/// when needed. `None` when reprojection fails.
fn to_pixels(
    points: Vec<(f64, f64, f64)>,
    grid: &GridSpec,
    transform: Option<&CoordTransform>,
) -> Option<Vec<(f64, f64)>> {
    let [x0, ew, _, y0, _, ns] = grid.geo_transform;
    let (mut xs, mut ys): (Vec<f64>, Vec<f64>) = points.into_iter().map(|(x, y, _)| (x, y)).unzip();
    if let Some(transform) = transform {
        transform.transform_coords(&mut xs, &mut ys, &mut []).ok()?;
    }
    Some(
        xs.into_iter()
            .zip(ys)
            .map(|(x, y)| ((x - x0) / ew, (y - y0) / ns))
            .collect(),
    )
}

fn collect_parts(
    geometry: &Geometry,
    grid: &GridSpec,
    transform: Option<&CoordTransform>,
    parts: &mut Parts,
) {
    let points = |g: &Geometry| {
        let mut out = Vec::new();
        g.get_points(&mut out);
        out
    };
    match geometry_type_flatten(geometry.geometry_type()) {
        OGRwkbGeometryType::wkbLineString | OGRwkbGeometryType::wkbLinearRing => {
            match to_pixels(points(geometry), grid, transform) {
                Some(line) if !line.is_empty() => parts.lines.push(line),
                Some(_) => {}
                None => parts.unsupported = true,
            }
        }
        OGRwkbGeometryType::wkbPolygon => {
            let mut rings = Vec::with_capacity(geometry.geometry_count());
            for i in 0..geometry.geometry_count() {
                match to_pixels(points(&geometry.get_geometry(i)), grid, transform) {
                    Some(ring) => rings.push(ring),
                    None => {
                        parts.unsupported = true;
                        return;
                    }
                }
            }
            if !rings.is_empty() {
                parts.polygons.push(rings);
            }
        }
        OGRwkbGeometryType::wkbMultiLineString
        | OGRwkbGeometryType::wkbMultiPolygon
        | OGRwkbGeometryType::wkbGeometryCollection => {
            for i in 0..geometry.geometry_count() {
                collect_parts(&geometry.get_geometry(i), grid, transform, parts);
            }
        }
        _ => parts.unsupported = true,
    }
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
#[derive(Debug, Clone, Serialize)]
pub struct MergeReport {
    /// Version of this document.
    pub report_version: u32,
    /// Deterministic part.
    pub run: MergeRecord,
    /// Execution part.
    pub execution: MergeExecution,
}

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

/// Execution part of `run.json`.
#[derive(Debug, Clone, Serialize)]
pub struct MergeExecution {
    /// Settings used.
    pub settings: ExecutionSettings,
    /// Memory and disk plan.
    pub plan: ResourcePlan,
    /// Seconds per phase.
    pub durations_s: BTreeMap<&'static str, f64>,
    /// Cache counters of the merge grid.
    pub store: StoreStats,
    /// Peak resident memory, bytes, where reported by the platform.
    pub peak_rss_bytes: Option<u64>,
    /// Versions of the engine and libraries.
    pub versions: BTreeMap<&'static str, String>,
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
    cancelled: &dyn Fn() -> bool,
) -> Result<(PathBuf, MergeReport)> {
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

    let started = Instant::now();
    for (item, plan) in prepared.job.stack.iter().zip(prepared.items.iter_mut()) {
        match item {
            StackItem::LandCover(item) => {
                paint_land_cover(item, plan, grid, &mut cells, cancelled)?
            }
            StackItem::Road(item) => {
                plan.features = Some(paint_road(item, plan, grid, &mut cells, cancelled)?);
            }
            StackItem::Barrier(item) => {
                plan.features = Some(paint_barrier(item, plan, grid, &mut cells, cancelled)?);
            }
        }
    }
    prepared
        .durations_s
        .insert("paint", started.elapsed().as_secs_f64());

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
    write_classes_csv(&staging.path().join(CLASSES_FILE), &result.classes)?;
    output::write_json(&staging.path().join("job.resolved.json"), &prepared.job)?;
    prepared
        .durations_s
        .insert("write_outputs", started.elapsed().as_secs_f64());

    let mut warnings = prepared.warnings.clone();
    for item in &prepared.items {
        if let Some(features) = &item.features
            && features.unsupported_geometry > 0
        {
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
    }
    let report = MergeReport {
        report_version: RUN_REPORT_VERSION,
        run: MergeRecord {
            job: prepared.job.clone(),
            inputs: prepared.fingerprints.clone(),
            grid: GridReport {
                cols: grid.cols,
                rows: grid.rows,
                geo_transform: grid.geo_transform,
                crs_wkt: grid.srs.to_wkt().unwrap_or_default(),
            },
            stack: prepared.items.clone(),
            result,
            warnings,
            deviations: deviations(),
        },
        execution: MergeExecution {
            settings: settings.clone(),
            plan: prepared.plan.clone(),
            durations_s: prepared.durations_s.clone(),
            store: cells.stats(),
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

fn paint_land_cover(
    item: &LandCoverItem,
    plan: &ItemPlan,
    grid: &GridSpec,
    cells: &mut TiledGrid<MergeCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let at = format!("stack item {} (land_cover)", plan.position);
    let (dataset, alignment) = open_class_raster(&item.path, grid, &at)?;
    let classes: BTreeMap<i64, (i32, u32)> = plan
        .classes
        .iter()
        .map(|c| (c.source_class, (c.class, c.rank)))
        .collect();
    for_each_raster_window(
        &dataset,
        &item.path,
        &alignment,
        grid,
        cancelled,
        |row, col, values| {
            for (k, value) in values.iter().enumerate() {
                let Some(&(class, rank)) = value.as_ref().and_then(|v| classes.get(v)) else {
                    continue;
                };
                let mut cell = cells.get(row, col + k)?;
                cell.paint(class, rank);
                cells.set(row, col + k, cell)?;
            }
            Ok(())
        },
    )
}

/// Apply `update` to every cell of the spans.
fn update_spans(
    cells: &mut TiledGrid<MergeCell>,
    spans: &[(usize, usize, usize)],
    update: impl Fn(&mut MergeCell),
) -> Result<()> {
    for &(row, c0, c1) in spans {
        for col in c0..=c1 {
            let mut cell = cells.get(row, col)?;
            update(&mut cell);
            cells.set(row, col, cell)?;
        }
    }
    Ok(())
}

fn paint_road(
    item: &RoadItem,
    plan: &ItemPlan,
    grid: &GridSpec,
    cells: &mut TiledGrid<MergeCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<FeatureCounts> {
    let at = format!("stack item {} (road)", plan.position);
    let source = VectorSource::open(
        &item.path,
        item.layer.as_deref(),
        item.filter.as_deref(),
        grid,
        &at,
    )?;
    let dataset = open_vector(&item.path)?;
    let mut layer = source.layer(&dataset)?;
    let class_index = layer
        .defn()
        .field_index(&item.class_field)
        .map_err(Error::gdal(&item.path))?;
    let classes: BTreeMap<i64, (i32, u32)> = plan
        .classes
        .iter()
        .map(|c| (c.source_class, (c.class, c.rank)))
        .collect();
    let mut counts = FeatureCounts::default();
    let mut spans = Vec::new();
    for feature in layer.features() {
        counts.read += 1;
        if counts.read % 4096 == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        let value = feature
            .field(class_index)
            .map_err(Error::gdal(&item.path))?;
        let Some(&(class, rank)) = value.and_then(integer_value).and_then(|v| classes.get(&v))
        else {
            counts.without_class += 1;
            continue;
        };
        let mut parts = Parts::default();
        if let Some(geometry) = feature.geometry() {
            collect_parts(geometry, grid, source.transform.as_ref(), &mut parts);
        }
        if parts.lines.is_empty() {
            counts.unsupported_geometry += 1;
            continue;
        }
        spans.clear();
        for line in &parts.lines {
            line_spans(line, grid.rows, grid.cols, &mut |r, c0, c1| {
                spans.push((r, c0, c1))
            });
        }
        update_spans(cells, &spans, |cell| {
            cell.paint(class, rank);
            cell.flags |= MergeCell::ROAD;
        })?;
        counts.used += 1;
        counts.lines += parts.lines.len() as u64;
    }
    Ok(counts)
}

fn paint_barrier(
    item: &BarrierItem,
    plan: &ItemPlan,
    grid: &GridSpec,
    cells: &mut TiledGrid<MergeCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<FeatureCounts> {
    let at = format!("stack item {} (barrier)", plan.position);
    let source = VectorSource::open(
        &item.path,
        item.layer.as_deref(),
        item.filter.as_deref(),
        grid,
        &at,
    )?;
    let dataset = open_vector(&item.path)?;
    let mut layer = source.layer(&dataset)?;
    let mut counts = FeatureCounts::default();
    let mut spans = Vec::new();
    for feature in layer.features() {
        counts.read += 1;
        if counts.read % 4096 == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        let mut parts = Parts::default();
        if let Some(geometry) = feature.geometry() {
            collect_parts(geometry, grid, source.transform.as_ref(), &mut parts);
        }
        if parts.lines.is_empty() && parts.polygons.is_empty() {
            counts.unsupported_geometry += 1;
            continue;
        }
        spans.clear();
        let mut push = |r, c0, c1| spans.push((r, c0, c1));
        for line in &parts.lines {
            line_spans(line, grid.rows, grid.cols, &mut push);
        }
        for polygon in &parts.polygons {
            polygon_spans(polygon, grid.rows, grid.cols, &mut push);
        }
        update_spans(cells, &spans, |cell| cell.flags |= MergeCell::BARRIER)?;
        counts.used += 1;
        counts.lines += parts.lines.len() as u64;
        counts.polygons += parts.polygons.len() as u64;
    }
    Ok(counts)
}

fn create_geotiff<T: gdal::raster::GdalType>(
    path: &Path,
    grid: &GridSpec,
    threads: usize,
    nodata: f64,
) -> Result<Dataset> {
    let driver = DriverManager::get_driver_by_name("GTiff").map_err(Error::gdal(path))?;
    let mut options = RasterCreationOptions::new();
    for (key, value) in [
        ("TILED", "YES".to_owned()),
        ("BLOCKXSIZE", BLOCK.to_string()),
        ("BLOCKYSIZE", BLOCK.to_string()),
        ("COMPRESS", "DEFLATE".to_owned()),
        ("PREDICTOR", "2".to_owned()),
        ("BIGTIFF", "IF_SAFER".to_owned()),
        ("NUM_THREADS", threads.max(1).to_string()),
    ] {
        options
            .set_name_value(key, &value)
            .map_err(Error::gdal(path))?;
    }
    let mut dataset = driver
        .create_with_band_type_with_options::<T, _>(path, grid.cols, grid.rows, 1, &options)
        .map_err(Error::gdal(path))?;
    dataset
        .set_geo_transform(&grid.geo_transform)
        .map_err(Error::gdal(path))?;
    dataset
        .set_spatial_ref(&grid.srs)
        .map_err(Error::gdal(path))?;
    dataset
        .rasterband(1)
        .and_then(|mut band| band.set_no_data_value(Some(nodata)))
        .map_err(Error::gdal(path))?;
    Ok(dataset)
}

fn write_block<T: gdal::raster::GdalType + Copy>(
    dataset: &Dataset,
    path: &Path,
    (col, row): (usize, usize),
    (width, height): (usize, usize),
    data: &mut Vec<T>,
) -> Result<()> {
    let mut buffer = Buffer::new((width, height), std::mem::take(data));
    let result = dataset.rasterband(1).and_then(|mut band| {
        band.write((col as isize, row as isize), (width, height), &mut buffer)
    });
    *data = buffer.into_shape_and_vec().1;
    data.clear();
    result.map_err(Error::gdal(path))
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
    let merged_path = dir.join(MERGED_FILE);
    let bridges_path = dir.join(BRIDGES_FILE);
    let merged_ds = create_geotiff::<i32>(&merged_path, grid, threads, f64::from(MERGED_NODATA))?;
    let bridges_ds = if clean_bridges {
        Some(create_geotiff::<u8>(
            &bridges_path,
            grid,
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
    for row0 in (0..rows).step_by(BLOCK) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let height = BLOCK.min(rows - row0);
        let top = row0.saturating_sub(1);
        let bottom = (row0 + height + 1).min(rows);
        for col0 in (0..cols).step_by(BLOCK) {
            let width = BLOCK.min(cols - col0);
            let left = col0.saturating_sub(1);
            let right = (col0 + width + 1).min(cols);
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
            for row in row0..row0 + height {
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
            let at = (col0, row0);
            let size = (width, height);
            write_block(&merged_ds, &merged_path, at, size, &mut merged_block)?;
            if let Some(ds) = &bridges_ds {
                write_block(ds, &bridges_path, at, size, &mut bridge_block)?;
            }
        }
    }
    // Barrier and uncovered cells, counted on whole rows.
    let mut row_cells = vec![MergeCell::EMPTY; BLOCK.min(cols)];
    for row in 0..rows {
        for col0 in (0..cols).step_by(row_cells.len()) {
            let width = row_cells.len().min(cols - col0);
            cells.read_row(row, col0, &mut row_cells[..width])?;
            for cell in &row_cells[..width] {
                if cell.is_barrier() {
                    summary.barrier_cells += 1;
                } else if cell.rank == MergeCell::NO_RANK {
                    summary.uncovered_cells += 1;
                }
            }
        }
    }
    merged_ds.close().map_err(Error::gdal(&merged_path))?;
    if let Some(ds) = bridges_ds {
        ds.close().map_err(Error::gdal(&bridges_path))?;
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

fn write_classes_csv(path: &Path, classes: &[MergedClass]) -> Result<()> {
    let mut text = String::from("class,label,stack_item,cells\n");
    for class in classes {
        let label = class
            .label
            .as_ref()
            .map(|l| csv_field(&serde_json::Value::String(l.clone())))
            .unwrap_or_default();
        text.push_str(&format!(
            "{},{label},{},{}\n",
            class.class, class.stack_item, class.cells
        ));
    }
    let mut file = std::fs::File::create(path).map_err(Error::file(path))?;
    file.write_all(text.as_bytes()).map_err(Error::file(path))
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
    fn attribute_classes() {
        assert_eq!(integer_value(FieldValue::IntegerValue(3)), Some(3));
        assert_eq!(integer_value(FieldValue::RealValue(4.0)), Some(4));
        assert_eq!(integer_value(FieldValue::RealValue(4.5)), None);
        assert_eq!(
            integer_value(FieldValue::StringValue(" 12 ".into())),
            Some(12)
        );
        assert_eq!(integer_value(FieldValue::StringValue("M".into())), None);
        assert!(output_class(i64::from(i32::MIN)).is_err());
        assert!(output_class(1 << 40).is_err());
    }
}
