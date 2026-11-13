//! Project Creator: a region and source files become a project that
//! computes at once (`docs/project-creator.md`).
//!
//! [`build`] works on local files only: the sources are named by a
//! [`Recipe`] (downloading them is another layer's job). It chooses the
//! projection and the grid, warps the rasters (DEM bilinear, land cover by
//! mode, population by sum, totals checked), extracts the OSM layers with
//! the [`presets`], and creates the project whole or not at all:
//!
//! 1. imports: DEM, land cover, population, roads, rivers, the area outside
//!    the traversable region as a barrier, facilities, zones;
//! 2. Merge land cover, the outside barrier first, so nothing outside the
//!    region can be crossed (contract 1);
//! 3. a travel-time job with the preset scenario, saved and checked: the
//!    project is published only once it is ready (contract 7);
//! 4. the manifest of what was used, in the project's metadata (contract 8).

pub mod presets;

use std::path::{Path, PathBuf};

use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use gdal::vector::{FieldValue, Geometry, LayerAccess, OGRFieldType, OGRwkbGeometryType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::Monitor;
use crate::class::DataClass;
use crate::error::{Error, Result};
use crate::grid::{Extent, GridSpec};
use crate::output::FLOAT_NODATA;
use crate::project::{ImportOptions, Project};
use crate::projection::{self, Distortion, LonLatBounds};
use crate::resources::ExecutionSettings;
use crate::vector::{VectorLayer, centroid, extract, parts_as_multi};
use crate::warp::{Resample, source_total, warp_to_grid};

pub use presets::Presets;

/// Distance distortion past which the projection is reported.
pub const DISTORTION_WARN: f64 = 0.005;
/// Distance distortion past which the region is refused.
pub const DISTORTION_MAX: f64 = 0.02;
/// Largest relative difference between the population of the sources and
/// of the grid over the same footprint.
pub const POPULATION_TOLERANCE: f64 = 0.005;
/// Most cells of a created grid.
pub const MAX_CELLS: u64 = 2_000_000_000;

/// Progress stages of [`build`].
pub const STAGE_GRID: &str = "grid";
/// Warping the rasters.
pub const STAGE_RASTERS: &str = "rasters";
/// Extracting the vector layers.
pub const STAGE_VECTORS: &str = "vectors";
/// Importing into the new project.
pub const STAGE_IMPORT: &str = "import";
/// Merge land cover and the readiness check.
pub const STAGE_MERGE: &str = "merge";

/// The region of a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Area {
    /// GeoJSON in WGS 84: a Polygon or MultiPolygon geometry, a Feature or
    /// a FeatureCollection (each feature a reporting zone, named by its
    /// `name` property).
    Geojson(serde_json::Value),
    /// A polygon layer file (any CRS); each feature a reporting zone, named
    /// by its first text field.
    Path(PathBuf),
}

/// The CRS of a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Crs {
    /// UTM when the region fits one zone well enough, otherwise a
    /// Transverse Mercator centred on it.
    #[default]
    Auto,
    /// An EPSG code (projected, metres).
    Epsg(u32),
    /// A WKT or PROJ definition (projected, metres).
    Definition(String),
    /// The CRS and lattice of a reference raster: rasters on that lattice
    /// import later without resampling.
    Reference(PathBuf),
}

/// Local source files of a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    /// DEM tiles (one product, one lattice).
    pub dem: Vec<PathBuf>,
    /// Land cover tiles (ESA WorldCover classes).
    pub land_cover: Vec<PathBuf>,
    /// Population count rasters (one per country).
    #[serde(default)]
    pub population: Vec<PathBuf>,
    /// OSM roads (HOT export layout: `highway`, `name`, `osm_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roads: Option<PathBuf>,
    /// OSM waterways (`waterway`, `name`, `osm_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waterways: Option<PathBuf>,
    /// OSM health facilities (`amenity`, `healthcare`, `name`, `osm_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facilities: Option<PathBuf>,
}

/// What to build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Must be 1.
    pub schema_version: u32,
    /// Project name.
    pub name: String,
    /// Region of interest.
    pub area: Area,
    /// Distance around the region that stays passable, metres.
    #[serde(default)]
    pub buffer_m: f64,
    /// Cell size, metres.
    pub resolution_m: f64,
    /// CRS.
    #[serde(default)]
    pub crs: Crs,
    /// Source files.
    pub sources: Sources,
}

impl Recipe {
    /// The recipe with its relative paths taken from `base` (a recipe
    /// file's directory).
    pub fn relative_to(mut self, base: &Path) -> Self {
        let fix = |p: &mut PathBuf| {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        };
        let s = &mut self.sources;
        s.dem
            .iter_mut()
            .chain(&mut s.land_cover)
            .chain(&mut s.population)
            .for_each(fix);
        [&mut s.roads, &mut s.waterways, &mut s.facilities]
            .into_iter()
            .flatten()
            .for_each(fix);
        if let Area::Path(p) = &mut self.area {
            fix(p);
        }
        if let Crs::Reference(p) = &mut self.crs {
            fix(p);
        }
        self
    }
}

/// A source file used, in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsedFile {
    /// File name.
    pub name: String,
    /// Size, bytes.
    pub bytes: u64,
    /// SHA-256.
    pub sha256: String,
}

/// Population totals over the grid's footprint.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PopulationCheck {
    /// Sum of the source cells centred on the grid.
    pub source: f64,
    /// Sum of the grid.
    pub grid: f64,
}

/// What a build did: kept in the project (`meta` key `creator`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    /// The recipe.
    pub recipe: Recipe,
    /// Engine version.
    pub accessmod: String,
    /// The projection chosen.
    pub crs_name: String,
    /// Its WKT.
    pub crs_wkt: String,
    /// Its distortion over the region.
    pub distortion: DistortionInfo,
    /// Grid: columns, rows, geotransform.
    pub grid: (usize, usize, [f64; 6]),
    /// Source files with their digests.
    pub files: Vec<UsedFile>,
    /// Preset names and versions.
    pub presets: Vec<(String, u32)>,
    /// Resampling of each raster layer.
    pub resampling: Vec<(String, String)>,
    /// Population totals, when there is a population.
    pub population: Option<PopulationCheck>,
    /// Features kept per layer, and left out.
    pub features: Vec<(String, u64)>,
    /// What the user should know.
    pub warnings: Vec<String>,
}

/// [`Distortion`] for the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DistortionInfo {
    /// Largest distance error.
    pub distance: f64,
    /// Largest area error.
    pub area: f64,
}

impl From<Distortion> for DistortionInfo {
    fn from(d: Distortion) -> Self {
        Self {
            distance: d.distance,
            area: d.area,
        }
    }
}

fn traditional(srs: &SpatialRef) -> SpatialRef {
    let mut srs = srs.clone();
    srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    srs
}

fn wgs84() -> Result<SpatialRef> {
    Ok(traditional(
        &SpatialRef::from_epsg(4326).map_err(Error::gdal("EPSG:4326"))?,
    ))
}

/// A zone of the region: its WGS 84 geometry and name.
struct Zone {
    geometry: Geometry,
    name: String,
}

/// The zones of `area`, in WGS 84.
/// Most polygons of an area.
pub const MAX_ZONES: usize = 10_000;
/// Most vertices of an area, all polygons together.
pub const MAX_VERTICES: usize = 5_000_000;

/// Vertices of a geometry, rings and parts included.
fn vertices(g: &Geometry) -> usize {
    match g.geometry_count() {
        0 => g.point_count(),
        n => (0..n).map(|i| vertices(&g.get_geometry(i))).sum(),
    }
}

fn zones(area: &Area, cancelled: crate::analysis::Cancelled<'_>) -> Result<Vec<Zone>> {
    let mut seen_vertices = 0usize;
    let mut polygon = |g: Geometry, name: String| -> Result<Zone> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let flat = gdal::vector::geometry_type_flatten(g.geometry_type());
        if !matches!(
            flat,
            OGRwkbGeometryType::wkbPolygon | OGRwkbGeometryType::wkbMultiPolygon
        ) {
            return Err(Error::invalid(format!(
                "the area holds a {}: polygons only",
                g.geometry_name()
            )));
        }
        // Union, buffer and difference need valid polygons: an invalid one
        // could silently lose the barrier outside the region.
        if !g.is_valid() {
            return Err(Error::invalid(format!(
                "area {name:?}: not a valid polygon (self-intersection, or a ring that crosses itself)"
            )));
        }
        seen_vertices += vertices(&g);
        if seen_vertices > MAX_VERTICES {
            return Err(Error::invalid(format!(
                "the area has more than {MAX_VERTICES} vertices: simplify it"
            )));
        }
        Ok(Zone { geometry: g, name })
    };
    let out = match area {
        Area::Geojson(value) => {
            let features: Vec<&serde_json::Value> = match value.get("type").and_then(|t| t.as_str())
            {
                Some("FeatureCollection") => value
                    .get("features")
                    .and_then(|f| f.as_array())
                    .map(|a| a.iter().collect())
                    .unwrap_or_default(),
                Some("Feature") => vec![value],
                _ => vec![],
            };
            if features.is_empty() {
                let g = Geometry::from_geojson(&value.to_string())
                    .map_err(|e| Error::invalid(format!("area: {e}")))?;
                vec![polygon(g, "area".to_owned())?]
            } else {
                let mut out = Vec::new();
                for (i, f) in features.iter().enumerate() {
                    let geometry = f
                        .get("geometry")
                        .ok_or_else(|| Error::invalid("area: a feature has no geometry"))?;
                    let g = Geometry::from_geojson(&geometry.to_string())
                        .map_err(|e| Error::invalid(format!("area: {e}")))?;
                    let name = f
                        .pointer("/properties/name")
                        .and_then(|n| n.as_str())
                        .map_or_else(|| format!("zone {}", i + 1), str::to_owned);
                    out.push(polygon(g, name)?);
                }
                out
            }
        }
        Area::Path(path) => {
            crate::project::check_self_contained(path)?;
            let dataset = gdal::Dataset::open(path).map_err(Error::gdal(path))?;
            let mut layer = dataset
                .layer(0)
                .map_err(|_| Error::invalid(format!("{}: no layer", path.display())))?;
            let srs = layer
                .spatial_ref()
                .ok_or_else(|| Error::invalid(format!("{}: no CRS", path.display())))?;
            let to_wgs84 =
                CoordTransform::new(&traditional(&srs), &wgs84()?).map_err(Error::gdal(path))?;
            let text_field = layer
                .defn()
                .fields()
                .position(|f| f.field_type() == OGRFieldType::OFTString);
            let mut out = Vec::new();
            for (i, feature) in layer.features().enumerate() {
                let Some(g) = feature.geometry() else {
                    continue;
                };
                let g = g.transform(&to_wgs84).map_err(Error::gdal(path))?;
                let name = text_field
                    .and_then(|f| feature.field_as_string(f).ok().flatten())
                    .unwrap_or_else(|| format!("zone {}", i + 1));
                out.push(polygon(g, name)?);
            }
            out
        }
    };
    if out.is_empty() {
        return Err(Error::invalid("the area holds no polygon"));
    }
    if out.len() > MAX_ZONES {
        return Err(Error::invalid(format!(
            "the area has more than {MAX_ZONES} polygons: merge or select them"
        )));
    }
    Ok(out)
}

fn lonlat_bounds<'a>(geometries: impl IntoIterator<Item = &'a Geometry>) -> LonLatBounds {
    let mut b = LonLatBounds {
        west: f64::INFINITY,
        south: f64::INFINITY,
        east: f64::NEG_INFINITY,
        north: f64::NEG_INFINITY,
    };
    for g in geometries {
        let e = g.envelope();
        b.west = b.west.min(e.MinX);
        b.east = b.east.max(e.MaxX);
        b.south = b.south.min(e.MinY);
        b.north = b.north.max(e.MaxY);
    }
    b
}

/// A chosen CRS: its name, the CRS, a lattice anchor (a cell corner) and
/// the cell size it imposes (a reference raster's).
struct ChosenCrs(String, SpatialRef, (f64, f64), Option<f64>);

impl From<(String, SpatialRef, (f64, f64), Option<f64>)> for ChosenCrs {
    fn from((name, srs, anchor, res): (String, SpatialRef, (f64, f64), Option<f64>)) -> Self {
        Self(name, srs, anchor, res)
    }
}

/// The CRS of the project, its name, and the lattice anchor.
fn choose_crs(crs: &Crs, bounds: &LonLatBounds) -> Result<ChosenCrs> {
    if let Some(problem) = bounds.check() {
        return Err(Error::invalid(problem));
    }
    Ok(ChosenCrs::from(match crs {
        Crs::Auto => {
            let (name, utm) = projection::utm(bounds)?;
            let fits =
                projection::distortion(&utm, bounds).is_ok_and(|d| d.distance <= DISTORTION_WARN);
            let (name, srs) = if fits {
                (name, utm)
            } else {
                projection::transverse_mercator(bounds)?
            };
            (name, srs, (0.0, 0.0), None)
        }
        Crs::Epsg(code) => (
            format!("EPSG:{code}"),
            SpatialRef::from_epsg(*code).map_err(Error::gdal(format!("EPSG:{code}")))?,
            (0.0, 0.0),
            None,
        ),
        Crs::Definition(text) => {
            let srs = SpatialRef::from_definition(text)
                .map_err(|e| Error::invalid(format!("CRS definition: {e}")))?;
            (
                srs.name().unwrap_or_else(|| "custom".to_owned()),
                srs,
                (0.0, 0.0),
                None,
            )
        }
        Crs::Reference(path) => {
            let (_, grid) = GridSpec::open_raster(path, "reference raster")?;
            let name = format!(
                "as {}",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            );
            let res = grid.geo_transform[1];
            if (res + grid.geo_transform[5]).abs() > res * 1e-9 {
                return Err(Error::invalid("the reference raster has no square cells"));
            }
            (
                name,
                grid.srs.clone(),
                (grid.geo_transform[0], grid.geo_transform[3]),
                Some(res),
            )
        }
    }))
}

fn file_record(path: &Path, cancelled: crate::analysis::Cancelled<'_>) -> Result<UsedFile> {
    let fingerprint = crate::report::fingerprints(&[("source", path)], cancelled)?
        .pop()
        .ok_or_else(|| Error::invalid("no fingerprint"))?;
    Ok(UsedFile {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        bytes: fingerprint.bytes,
        sha256: fingerprint.sha256,
    })
}

fn field_text(feature: &gdal::vector::Feature<'_>, name: &str) -> Option<String> {
    let index = feature.field_index(name).ok()?;
    feature
        .field_as_string(index)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

fn osm_id(feature: &gdal::vector::Feature<'_>) -> Option<FieldValue> {
    let index = feature.field_index("osm_id").ok()?;
    feature
        .field_as_integer64(index)
        .ok()
        .flatten()
        .map(FieldValue::Integer64Value)
}

/// Cells whose centre lies in `region` (grid CRS), and how many of them
/// `path` (on the grid) leaves at NoData: a source tile missing under the
/// region (contract 2).
fn missing_inside(
    path: &Path,
    grid: &GridSpec,
    region: &Geometry,
    cancelled: crate::analysis::Cancelled<'_>,
) -> Result<(u64, u64)> {
    let dataset = gdal::Dataset::open(path).map_err(Error::gdal(path))?;
    let band = dataset.rasterband(1).map_err(Error::gdal(path))?;
    let nodata = band.no_data_value();
    let polygons: Vec<Geometry> = match gdal::vector::geometry_type_flatten(region.geometry_type())
    {
        OGRwkbGeometryType::wkbPolygon => vec![region.clone()],
        _ => (0..region.geometry_count())
            .map(|i| region.get_geometry(i).clone())
            .collect(),
    };
    let (mut inside, mut missing) = (0u64, 0u64);
    let mut failure = None;
    for polygon in polygons {
        let rings: Vec<Vec<(f64, f64)>> = (0..polygon.geometry_count())
            .map(|i| {
                let mut points = Vec::new();
                polygon.get_geometry(i).get_points(&mut points);
                points
                    .into_iter()
                    .map(|(x, y, _)| grid.to_pixel(x, y))
                    .collect()
            })
            .collect();
        accessmod_core::raster::polygon_coverage(
            &rings,
            grid.rows,
            grid.cols,
            &mut |row, first, fractions| {
                if cancelled() {
                    failure = Some(Error::Cancelled);
                    return std::ops::ControlFlow::Break(());
                }
                let read = band.read_as::<f64>(
                    (first as isize, row as isize),
                    (fractions.len(), 1),
                    (fractions.len(), 1),
                    None,
                );
                match read {
                    Ok(buffer) => {
                        for (value, fraction) in buffer.into_shape_and_vec().1.iter().zip(fractions)
                        {
                            // The cell centre is inside when most of the cell is
                            // (cells on the boundary count half the time).
                            if *fraction >= 0.5 {
                                inside += 1;
                                if Some(*value) == nodata || value.is_nan() {
                                    missing += 1;
                                }
                            }
                        }
                        std::ops::ControlFlow::Continue(())
                    }
                    Err(e) => {
                        failure = Some(Error::gdal(path)(e));
                        std::ops::ControlFlow::Break(())
                    }
                }
            },
        );
        if let Some(e) = failure.take() {
            return Err(e);
        }
    }
    Ok((inside, missing))
}

/// Build the project of `recipe` into `target` (a new directory), from
/// local files: the project, and the manifest of the build.
pub fn build(
    recipe: &Recipe,
    target: &Path,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<(Project, Manifest)> {
    if recipe.schema_version != 1 {
        return Err(Error::invalid("recipe schema_version must be 1"));
    }
    if !(recipe.buffer_m.is_finite() && recipe.buffer_m >= 0.0) {
        return Err(Error::invalid("buffer_m must be 0 or more metres"));
    }
    if recipe.sources.dem.is_empty() || recipe.sources.land_cover.is_empty() {
        return Err(Error::invalid("a DEM and a land cover are needed"));
    }
    let presets = Presets::shipped()?;
    let cancelled = monitor.cancelled();
    monitor.stage(STAGE_GRID);
    let zones = zones(&recipe.area, cancelled)?;
    let bounds = lonlat_bounds(zones.iter().map(|z| &z.geometry));
    let ChosenCrs(crs_name, srs, anchor, reference_res) = choose_crs(&recipe.crs, &bounds)?;
    if let Some(res) = reference_res
        && (res - recipe.resolution_m).abs() > res * 1e-9
    {
        return Err(Error::invalid(format!(
            "the reference raster has {res} m cells; the resolution must match"
        )));
    }
    let mut warnings = Vec::new();
    // The region and its buffer in the project CRS.
    let to_project =
        CoordTransform::new(&wgs84()?, &traditional(&srs)).map_err(Error::gdal("projection"))?;
    let mut region: Option<Geometry> = None;
    let mut projected_zones = Vec::new();
    for zone in &zones {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let g = zone
            .geometry
            .transform(&to_project)
            .map_err(|_| Error::invalid("the area does not project into the chosen CRS"))?;
        region = Some(match region {
            None => g.clone(),
            Some(r) => r
                .union(&g)
                .ok_or_else(|| Error::invalid("the area polygons cannot be merged"))?,
        });
        projected_zones.push((g, zone.name.clone()));
    }
    let region = region.ok_or_else(|| Error::invalid("the area holds no polygon"))?;
    let traversable = if recipe.buffer_m > 0.0 {
        region
            .buffer(recipe.buffer_m, 8)
            .map_err(Error::gdal("buffer"))?
    } else {
        region.clone()
    };
    // Distortion over the traversable extent, back in WGS 84.
    let to_wgs84 =
        CoordTransform::new(&traditional(&srs), &wgs84()?).map_err(Error::gdal("projection"))?;
    let extent_lonlat = lonlat_bounds([&traversable
        .transform(&to_wgs84)
        .map_err(|_| Error::invalid("the buffered area leaves the CRS"))?]);
    let distortion = projection::distortion(&srs, &extent_lonlat)?;
    if distortion.distance > DISTORTION_MAX {
        return Err(Error::invalid(format!(
            "{crs_name} distorts distances by up to {:.2} % over this region (limit {:.0} %): \
             split the region or choose another CRS",
            distortion.distance * 100.0,
            DISTORTION_MAX * 100.0
        )));
    }
    if distortion.distance > DISTORTION_WARN {
        warnings.push(format!(
            "{crs_name} distorts distances by up to {:.2} % over this region",
            distortion.distance * 100.0
        ));
    }
    let e = traversable.envelope();
    let grid = GridSpec::from_extent(
        Extent {
            xmin: e.MinX,
            ymin: e.MinY,
            xmax: e.MaxX,
            ymax: e.MaxY,
        },
        recipe.resolution_m,
        srs.clone(),
        anchor,
        MAX_CELLS,
    )
    .map_err(|issues| Error::Validation { issues })?;

    // Work files next to the project, removed at the end.
    let parent = target
        .parent()
        .ok_or_else(|| Error::invalid("a project directory needs a parent"))?;
    let staging = parent.join(format!(
        ".creator-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&staging).map_err(Error::file(&staging))?;
    let result = (|| {
        let work = |name: &str| staging.join(name);
        // Rasters.
        monitor.stage(STAGE_RASTERS);
        let threads = settings.threads;
        warp_to_grid::<f32>(
            &recipe.sources.dem,
            &grid,
            Resample::Bilinear,
            &work("dem.tif"),
            -9999.0,
            |v| v as f32,
            threads,
            cancelled,
        )?;
        let lc_nodata = presets.land_cover.nodata as f64;
        warp_to_grid::<i32>(
            &recipe.sources.land_cover,
            &grid,
            Resample::Mode,
            &work("land_cover.tif"),
            lc_nodata,
            |v| v.round() as i32,
            threads,
            cancelled,
        )?;
        // A tile missing under the region would leave silent holes.
        for (name, file) in [("DEM", "dem.tif"), ("land cover", "land_cover.tif")] {
            let (inside, missing) = missing_inside(&work(file), &grid, &traversable, cancelled)?;
            if missing > 0 {
                return Err(Error::invalid(format!(
                    "the {name} sources leave {missing} of {inside} cells of the region without data \
                     ({:.1} %): add the missing tiles",
                    missing as f64 * 100.0 / inside.max(1) as f64
                )));
            }
        }
        let population = if recipe.sources.population.is_empty() {
            None
        } else {
            let written = warp_to_grid::<f64>(
                &recipe.sources.population,
                &grid,
                Resample::Sum,
                &work("population.tif"),
                FLOAT_NODATA,
                |v| v,
                threads,
                cancelled,
            )?;
            let source = source_total(&recipe.sources.population, &grid, cancelled)?;
            let check = PopulationCheck {
                source,
                grid: written.sum,
            };
            let relative = if source > 0.0 {
                (written.sum - source).abs() / source
            } else {
                written.sum.abs()
            };
            if relative > POPULATION_TOLERANCE {
                return Err(Error::invalid(format!(
                    "population: the grid holds {:.0} people where the sources hold {source:.0} \
                     over the same area ({:.2} % apart, limit {:.1} %)",
                    written.sum,
                    relative * 100.0,
                    POPULATION_TOLERANCE * 100.0
                )));
            }
            Some(check)
        };

        // Vectors.
        monitor.stage(STAGE_VECTORS);
        let mut features = Vec::new();
        // The grid outside the region: a barrier. An overlay failure must
        // not pass for "nothing outside" (the region would leak).
        let outside = {
            let frame = grid.outline()?;
            if traversable.contains(&frame) {
                None
            } else {
                Some(frame.difference(&traversable).ok_or_else(|| {
                    Error::invalid(
                        "the area outline cannot be cut from the grid (invalid geometry)",
                    )
                })?)
            }
        };
        let mut outside_file = None;
        if let Some(outside) =
            outside.and_then(|g| parts_as_multi(&g, OGRwkbGeometryType::wkbMultiPolygon))
        {
            let path = work("outside.fgb");
            let mut writer = crate::vector::FeatureWriter::create(
                &path,
                "outside",
                &grid.srs,
                OGRwkbGeometryType::wkbMultiPolygon,
                &[],
            )?;
            writer.write(outside, &[])?;
            writer.close()?;
            outside_file = Some(path);
        }
        let mut roads_file = None;
        if let Some(source) = &recipe.sources.roads {
            let input = VectorLayer::open(source, None, None, &grid, "roads")?;
            let path = work("roads.fgb");
            let mut left_out = 0u64;
            let kept = extract(
                &input,
                Some(&traversable),
                &grid.srs,
                OGRwkbGeometryType::wkbMultiLineString,
                &[
                    ("road_class", OGRFieldType::OFTInteger),
                    ("label", OGRFieldType::OFTString),
                    ("highway", OGRFieldType::OFTString),
                    ("name", OGRFieldType::OFTString),
                    ("osm_id", OGRFieldType::OFTInteger64),
                ],
                &path,
                cancelled,
                |feature, geometry| {
                    let highway = field_text(feature, "highway")?;
                    let fields = |name: &str| field_text(feature, name);
                    let Some(rule) = presets.roads.rule(&highway, &fields) else {
                        left_out += 1;
                        return None;
                    };
                    let geometry =
                        parts_as_multi(&geometry, OGRwkbGeometryType::wkbMultiLineString)?;
                    Some((
                        geometry,
                        vec![
                            Some(FieldValue::IntegerValue(rule.class as i32)),
                            Some(FieldValue::StringValue(rule.label.clone())),
                            Some(FieldValue::StringValue(highway)),
                            field_text(feature, "name").map(FieldValue::StringValue),
                            osm_id(feature),
                        ],
                    ))
                },
            )?;
            features.push(("roads".to_owned(), kept));
            features.push(("roads left out (preset)".to_owned(), left_out));
            roads_file = (kept > 0).then_some(path);
        }
        let mut rivers_file = None;
        if let Some(source) = &recipe.sources.waterways {
            let input = VectorLayer::open(source, None, None, &grid, "waterways")?;
            let path = work("rivers.fgb");
            let kept = extract(
                &input,
                Some(&traversable),
                &grid.srs,
                OGRwkbGeometryType::wkbMultiLineString,
                &[
                    ("label", OGRFieldType::OFTString),
                    ("name", OGRFieldType::OFTString),
                    ("osm_id", OGRFieldType::OFTInteger64),
                ],
                &path,
                cancelled,
                |feature, geometry| {
                    let waterway = field_text(feature, "waterway")?;
                    let rule = presets
                        .barriers
                        .rules
                        .iter()
                        .find(|r| r.waterway.contains(&waterway))?;
                    let geometry =
                        parts_as_multi(&geometry, OGRwkbGeometryType::wkbMultiLineString)?;
                    Some((
                        geometry,
                        vec![
                            Some(FieldValue::StringValue(rule.label.clone())),
                            field_text(feature, "name").map(FieldValue::StringValue),
                            osm_id(feature),
                        ],
                    ))
                },
            )?;
            features.push(("rivers".to_owned(), kept));
            rivers_file = (kept > 0).then_some(path);
        }
        let mut facilities_file = None;
        if let Some(source) = &recipe.sources.facilities {
            let input = VectorLayer::open(source, None, None, &grid, "facilities")?;
            let path = work("facilities.fgb");
            let mut seen = std::collections::HashSet::new();
            let kept = extract(
                &input,
                // Not clipped: a facility polygon keeps its own centroid,
                // whatever the region cuts of it.
                None,
                &grid.srs,
                OGRwkbGeometryType::wkbPoint,
                &[
                    ("osm_id", OGRFieldType::OFTInteger64),
                    ("name", OGRFieldType::OFTString),
                    ("amenity", OGRFieldType::OFTString),
                    ("healthcare", OGRFieldType::OFTString),
                ],
                &path,
                cancelled,
                |feature, geometry| {
                    let keep = presets.facilities.keep.iter().any(|(field, values)| {
                        field_text(feature, field).is_some_and(|v| values.contains(&v))
                    });
                    let id = osm_id(feature)?;
                    if !keep || !seen.insert(format!("{id:?}")) {
                        return None;
                    }
                    let point = match gdal::vector::geometry_type_flatten(geometry.geometry_type())
                    {
                        OGRwkbGeometryType::wkbPoint => geometry,
                        _ => centroid(&geometry)?,
                    };
                    if !traversable.intersects(&point) {
                        return None;
                    }
                    Some((
                        point,
                        vec![
                            Some(id),
                            field_text(feature, "name").map(FieldValue::StringValue),
                            field_text(feature, "amenity").map(FieldValue::StringValue),
                            field_text(feature, "healthcare").map(FieldValue::StringValue),
                        ],
                    ))
                },
            )?;
            features.push(("facilities".to_owned(), kept));
            facilities_file = (kept > 0).then_some(path);
        }
        // Reporting zones.
        let zones_file = work("zones.fgb");
        {
            let mut writer = crate::vector::FeatureWriter::create(
                &zones_file,
                "zones",
                &grid.srs,
                OGRwkbGeometryType::wkbMultiPolygon,
                &[
                    ("zone_id", OGRFieldType::OFTInteger),
                    ("name", OGRFieldType::OFTString),
                ],
            )?;
            for (i, (g, name)) in projected_zones.iter().enumerate() {
                if let Some(g) = parts_as_multi(g, OGRwkbGeometryType::wkbMultiPolygon) {
                    writer.write(
                        g,
                        &[
                            Some(FieldValue::IntegerValue(i as i32 + 1)),
                            Some(FieldValue::StringValue(name.clone())),
                        ],
                    )?;
                }
            }
            writer.close()?;
        }

        // Ready means a travel time can run (contract 7): it needs facilities.
        if facilities_file.is_none() {
            return Err(Error::invalid(
                "no health facility in the region: give a facilities layer (an official list, \
                 or OpenStreetMap where it has some)",
            ));
        }
        let mut files = Vec::new();
        for path in recipe
            .sources
            .dem
            .iter()
            .chain(&recipe.sources.land_cover)
            .chain(&recipe.sources.population)
            .chain(recipe.sources.roads.iter())
            .chain(recipe.sources.waterways.iter())
            .chain(recipe.sources.facilities.iter())
            .chain(match &recipe.area {
                Area::Path(p) => Some(p),
                Area::Geojson(_) => None,
            })
            .chain(match &recipe.crs {
                Crs::Reference(p) => Some(p),
                _ => None,
            })
        {
            files.push(file_record(path, cancelled)?);
        }
        let mut manifest = Manifest {
            recipe: recipe.clone(),
            accessmod: env!("CARGO_PKG_VERSION").to_owned(),
            crs_name: crs_name.clone(),
            crs_wkt: grid.srs.to_wkt().unwrap_or_default(),
            distortion: distortion.into(),
            grid: (grid.cols, grid.rows, grid.geo_transform),
            files,
            presets: vec![
                (
                    presets.land_cover.preset.clone(),
                    presets.land_cover.version,
                ),
                (presets.roads.preset.clone(), presets.roads.version),
                (presets.barriers.preset.clone(), presets.barriers.version),
                (
                    presets.facilities.preset.clone(),
                    presets.facilities.version,
                ),
            ],
            resampling: [
                ("dem", "bilinear"),
                ("land_cover", "mode (full resolution)"),
                ("population", "sum, checked over the grid footprint"),
            ]
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect(),
            population,
            features,
            warnings: warnings.clone(),
        };

        // The project, whole or not at all.
        monitor.stage(STAGE_IMPORT);
        let tags = |t: &[&str]| t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let project = Project::create_with(
            target,
            &recipe.name,
            "Created from a region (Project Creator)",
            &work("dem.tif"),
            settings,
            cancelled,
            |project| {
                let import = |path: &Path, class: DataClass, t: &[&str]| {
                    project.import(
                        path,
                        class,
                        &tags(t),
                        &ImportOptions::default(),
                        settings,
                        cancelled,
                    )
                };
                let lc = import(
                    &work("land_cover.tif"),
                    DataClass::LandCover,
                    &["worldcover"],
                )?;
                if population.is_some() {
                    import(
                        &work("population.tif"),
                        DataClass::Population,
                        &["worldpop"],
                    )?;
                }
                let roads = roads_file
                    .as_deref()
                    .map(|p| import(p, DataClass::Road, &["osm"]))
                    .transpose()?;
                let rivers = rivers_file
                    .as_deref()
                    .map(|p| import(p, DataClass::Barrier, &["osm", "rivers"]))
                    .transpose()?;
                let outside = outside_file
                    .as_deref()
                    .map(|p| import(p, DataClass::Barrier, &["outside", "region"]))
                    .transpose()?;
                let facilities = facilities_file
                    .as_deref()
                    .map(|p| import(p, DataClass::Facility, &["osm"]))
                    .transpose()?;
                import(&zones_file, DataClass::Zone, &["area"])?;

                // Merge: the outside first, then rivers, roads, land cover.
                monitor.stage(STAGE_MERGE);
                let dem = project.dataset(project.info()?.dem)?;
                let mut stack = Vec::new();
                for barrier in [&outside, &rivers].into_iter().flatten() {
                    stack.push(serde_json::json!({"type": "barrier", "path": barrier.file}));
                }
                if let Some(roads) = &roads {
                    stack.push(serde_json::json!({"type": "road", "path": roads.file, "class_field": "road_class", "label_field": "label", "add_1000": true}));
                }
                let labels: Vec<serde_json::Value> = presets
                    .land_cover
                    .classes
                    .iter()
                    .map(|c| serde_json::json!({"class": c.value, "label": c.label}))
                    .collect();
                stack.push(
                    serde_json::json!({"type": "land_cover", "path": lc.file, "labels": labels}),
                );
                let merge =
                    serde_json::json!({"schema_version": 1, "dem": dem.file, "stack": stack});
                let run = project.run_named(
                    "merge-land-cover",
                    &merge,
                    &tags(&["creator"]),
                    false,
                    settings,
                    monitor,
                )?;
                if run.status != crate::project::RunStatus::Done {
                    return Err(Error::invalid(format!(
                        "merge land cover: {}",
                        run.outcome.unwrap_or_default()
                    )));
                }
                let merged = run
                    .datasets
                    .iter()
                    .filter_map(|&id| project.dataset(id).ok())
                    .find(|d| d.class == DataClass::LandCoverMerged)
                    .ok_or_else(|| Error::invalid("merge land cover wrote no merged land cover"))?;

                // The scenario of the presets, and a travel-time job.
                // Only the classes the merged land cover holds (when its
                // classes are known), so the check warns about real issues.
                let present: Option<Vec<i64>> = match &merged.summary {
                    crate::project::Summary::Raster(p) => p
                        .stats
                        .values
                        .as_ref()
                        .map(|v| v.iter().map(|c| c.value).collect()),
                    _ => None,
                };
                let holds = |class: i64| present.as_ref().is_none_or(|p| p.contains(&class));
                let mut scenario: Vec<serde_json::Value> = presets
                    .land_cover
                    .classes
                    .iter()
                    .filter(|c| holds(c.value))
                    .map(|c| serde_json::json!({"class": c.value, "mode": c.mode, "speed_kmh": c.speed_kmh, "label": c.label}))
                    .collect();
                if roads.is_some() {
                    scenario.extend(
                        presets
                            .roads
                            .rules
                            .iter()
                            .filter(|r| holds(r.class + 1000))
                            .map(|r| serde_json::json!({"class": r.class + 1000, "mode": r.mode, "speed_kmh": r.speed_kmh, "label": r.label})),
                    );
                }
                if let Some(facilities) = &facilities {
                    let job = serde_json::json!({
                        "schema_version": 1,
                        "dem": dem.file,
                        "land_cover": merged.file,
                        "facilities": {"path": facilities.file, "id_field": "osm_id"},
                        "scenario": scenario,
                        "method": "anisotropic",
                        "direction": "from_facilities",
                        "neighbourhood": 8,
                        // OSM facilities on a barrier are listed, never
                        // moved (contract 6): the job runs without them.
                        "facility_issues": "skip"
                    });
                    let warnings = project.check_named("travel-time", &job)?;
                    manifest
                        .warnings
                        .extend(warnings.into_iter().map(|w| w.message));
                    project.save_job(None, "travel-time", "Travel time (Project Creator)", &job)?;
                }
                let text =
                    serde_json::to_string(&manifest).map_err(|e| Error::invalid(e.to_string()))?;
                project.set_meta("creator", &text)
            },
        )?;
        Ok((project, manifest))
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}
