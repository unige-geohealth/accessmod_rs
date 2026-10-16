//! Vector layers read onto the grid: layer and attribute filter checked up
//! front, explicit reprojection to the DEM CRS, geometries split into line
//! and polygon parts in grid pixel coordinates (see
//! `accessmod_core::raster`).

use std::path::{Path, PathBuf};

use accessmod_core::raster::{line_spans, polygon_spans};
use accessmod_core::store::{CellCodec, TiledGrid};
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use gdal::vector::{
    Feature, FieldValue, Geometry, Layer, LayerAccess, OGRwkbGeometryType, geometry_type_flatten,
};
use gdal::{Dataset, DatasetOptions, GdalOpenFlags};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::grid::GridSpec;

/// Why coordinates cannot be placed on the grid.
pub const NOT_REPROJECTABLE: &str = "cannot be reprojected to the DEM CRS";
/// Coordinates with NaN or infinite values.
pub const NON_FINITE: &str = "has non-finite coordinates";

/// A vector layer used by a job, checked, with its transform to the grid
/// CRS. Reading reopens the dataset ([`VectorLayer::dataset`]), so the
/// layer can be read several times.
#[derive(Debug)]
pub struct VectorLayer {
    /// Layer name.
    pub name: String,
    role: String,
    path: PathBuf,
    filter: Option<String>,
    transform: Option<CoordTransform>,
}

/// A line or polygon of a geometry, in pixel coordinates of the grid.
#[derive(Debug)]
pub enum Part {
    /// Line string or ring.
    Line(Vec<(f64, f64)>),
    /// Exterior ring then holes.
    Polygon(Vec<Vec<(f64, f64)>>),
}

fn open_dataset(path: &Path) -> Result<Dataset> {
    let options = DatasetOptions {
        open_flags: GdalOpenFlags::GDAL_OF_VECTOR | GdalOpenFlags::GDAL_OF_READONLY,
        ..DatasetOptions::default()
    };
    Dataset::open_ex(path, options).map_err(Error::gdal(path))
}

fn traditional(mut srs: SpatialRef) -> SpatialRef {
    srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    srs
}

impl VectorLayer {
    /// Check layer `layer` (the first when `None`) of `path`: it exists, has
    /// a CRS, and `filter` (OGR SQL `WHERE`) is valid. `role` names the
    /// layer in messages.
    pub fn open(
        path: &Path,
        layer: Option<&str>,
        filter: Option<&str>,
        grid: &GridSpec,
        role: &str,
    ) -> Result<Self> {
        let dataset = open_dataset(path)?;
        let invalid = |issue: String| Error::invalid(format!("{role} {}: {issue}", path.display()));
        let mut found = match layer {
            Some(name) => dataset
                .layer_by_name(name)
                .map_err(|_| invalid(format!("no layer named {name:?}")))?,
            None => dataset
                .layer(0)
                .map_err(|_| invalid("has no layer".into()))?,
        };
        let name = found.name();
        let layer_srs = found
            .spatial_ref()
            .ok_or_else(|| invalid(format!("layer {name:?} has no CRS")))?;
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
                name.replace('"', "\"\"")
            );
            dataset
                .execute_sql(&query, None, gdal::vector::sql::Dialect::DEFAULT)
                .map_err(|e| invalid(format!("invalid filter {filter:?} ({e})")))?;
        }
        Ok(Self {
            name,
            role: role.to_owned(),
            path: path.to_path_buf(),
            filter: filter.map(str::to_owned),
            transform,
        })
    }

    /// Whether coordinates are reprojected to the grid CRS.
    pub fn reprojected(&self) -> bool {
        self.transform.is_some()
    }

    /// Validation error about this layer.
    pub fn invalid(&self, issue: String) -> Error {
        Error::invalid(format!("{} {}: {issue}", self.role, self.path.display()))
    }

    /// The dataset, opened again for reading.
    pub fn dataset(&self) -> Result<Dataset> {
        open_dataset(&self.path)
    }

    /// The layer of `dataset` (from [`VectorLayer::dataset`]), with the
    /// attribute filter set.
    pub fn layer<'a>(&self, dataset: &'a Dataset) -> Result<Layer<'a>> {
        let mut layer = dataset
            .layer_by_name(&self.name)
            .map_err(Error::gdal(&self.path))?;
        if let Some(filter) = &self.filter {
            layer
                .set_attribute_filter(filter)
                .map_err(Error::gdal(&self.path))?;
        }
        Ok(layer)
    }

    /// Index of field `field` in `layer`, or a validation error.
    pub fn field_index(&self, layer: &Layer<'_>, field: &str) -> Result<usize> {
        layer
            .defn()
            .field_index(field)
            .map_err(|_| self.invalid(format!("layer {:?} has no field {field:?}", self.name)))
    }

    /// Reproject coordinates to the grid CRS in place, when needed.
    pub fn to_grid_crs(
        &self,
        xs: &mut [f64],
        ys: &mut [f64],
    ) -> std::result::Result<(), &'static str> {
        match &self.transform {
            Some(transform) => transform
                .transform_coords(xs, ys, &mut [])
                .map_err(|_| NOT_REPROJECTABLE),
            None => Ok(()),
        }
    }

    /// Grid pixel coordinates of `points`, or why they have none.
    fn to_pixels(
        &self,
        points: Vec<(f64, f64, f64)>,
        grid: &GridSpec,
    ) -> std::result::Result<Vec<(f64, f64)>, &'static str> {
        let (mut xs, mut ys): (Vec<f64>, Vec<f64>) =
            points.into_iter().map(|(x, y, _)| (x, y)).unzip();
        self.to_grid_crs(&mut xs, &mut ys)?;
        let pixels: Vec<(f64, f64)> = xs
            .into_iter()
            .zip(ys)
            .map(|(x, y)| grid.to_pixel(x, y))
            .collect();
        if pixels.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
            Ok(pixels)
        } else {
            Err(NON_FINITE)
        }
    }

    /// Visit the line and polygon parts of `geometry` one at a time, so
    /// that only one part's coordinates are held. Multi-geometries and
    /// collections are split; other parts (points, curves) are counted in
    /// `unsupported`. A part that cannot be reprojected, or has non-finite
    /// coordinates, is an error built by `problem`.
    pub fn for_each_part(
        &self,
        geometry: &Geometry,
        grid: &GridSpec,
        problem: &dyn Fn(&str) -> Error,
        unsupported: &mut u64,
        visit: &mut dyn FnMut(Part) -> Result<()>,
    ) -> Result<()> {
        let points = |g: &Geometry| {
            let mut out = Vec::new();
            g.get_points(&mut out);
            out
        };
        match geometry_type_flatten(geometry.geometry_type()) {
            OGRwkbGeometryType::wkbLineString | OGRwkbGeometryType::wkbLinearRing => {
                let line = self.to_pixels(points(geometry), grid).map_err(problem)?;
                if !line.is_empty() {
                    visit(Part::Line(line))?;
                }
            }
            OGRwkbGeometryType::wkbPolygon => {
                let mut rings = Vec::with_capacity(geometry.geometry_count());
                for i in 0..geometry.geometry_count() {
                    rings.push(
                        self.to_pixels(points(&geometry.get_geometry(i)), grid)
                            .map_err(problem)?,
                    );
                }
                if !rings.is_empty() {
                    visit(Part::Polygon(rings))?;
                }
            }
            OGRwkbGeometryType::wkbMultiLineString
            | OGRwkbGeometryType::wkbMultiPolygon
            | OGRwkbGeometryType::wkbGeometryCollection => {
                for i in 0..geometry.geometry_count() {
                    self.for_each_part(
                        &geometry.get_geometry(i),
                        grid,
                        problem,
                        unsupported,
                        visit,
                    )?;
                }
            }
            _ => *unsupported += 1,
        }
        Ok(())
    }

    /// Error about feature `fid` (or 0-based position `index`) of this
    /// layer, built from the reason given.
    pub fn feature_problem(&self, fid: Option<u64>, index: u64) -> impl Fn(&str) -> Error {
        let which = fid.map_or_else(|| format!("#{index}"), |fid| format!("FID {fid}"));
        let role = self.role.clone();
        move |why: &str| {
            Error::invalid(format!(
                "{role} feature {which}: {why}; fix it or leave it out with \"where\""
            ))
        }
    }
}

/// Integer value of an attribute: integers, whole reals and integer text.
pub fn integer_attribute(value: FieldValue) -> Option<i64> {
    match value {
        FieldValue::IntegerValue(v) => Some(i64::from(v)),
        FieldValue::Integer64Value(v) => Some(v),
        FieldValue::RealValue(v) if v.fract() == 0.0 && v.abs() < 9.0e15 => Some(v as i64),
        FieldValue::StringValue(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Which parts of the features are burnt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Burn {
    /// Lines only (as `v.to.rast type=line`); polygons are ignored.
    Lines,
    /// Lines (every cell touched) and polygons (cell centre inside).
    LinesAndPolygons,
}

/// Feature counts of a burnt layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FeatureCounts {
    /// Features read (after the attribute filter).
    pub read: u64,
    /// Features that burnt at least one cell.
    pub used: u64,
    /// Features the caller left out (roads: no usable class, or not in
    /// `classes`).
    pub without_class: u64,
    /// Features without a part to burn: no geometry, points, curves, or
    /// polygons when only lines are burnt.
    pub unsupported_geometry: u64,
    /// Features with parts that burn no cell (outside the grid, or polygons
    /// too small to hold a cell centre).
    pub outside_grid: u64,
    /// Line parts burnt.
    pub lines: u64,
    /// Polygon parts burnt.
    pub polygons: u64,
}

impl FeatureCounts {
    /// Count one feature with `lines` and `polygons` parts.
    fn tally(&mut self, lines: u64, polygons: u64, burnt: bool) {
        self.lines += lines;
        self.polygons += polygons;
        if lines + polygons == 0 {
            self.unsupported_geometry += 1;
        } else if burnt {
            self.used += 1;
        } else {
            self.outside_grid += 1;
        }
    }
}

/// Apply `update` to every cell `part` burns (see `accessmod_core::raster`);
/// `true` when it burns at least one cell of the grid.
pub fn burn_part<T: CellCodec>(
    part: &Part,
    grid: &GridSpec,
    cells: &mut TiledGrid<T>,
    update: &dyn Fn(&mut T),
    cancelled: &dyn Fn() -> bool,
) -> Result<bool> {
    let mut failure = None;
    let mut burnt = false;
    let mut visit = |row: usize, c0: usize, c1: usize| {
        if failure.is_some() {
            return;
        }
        if cancelled() {
            failure = Some(Error::Cancelled);
            return;
        }
        burnt = true;
        for col in c0..=c1 {
            let result = cells.get(row, col).and_then(|mut cell| {
                update(&mut cell);
                cells.set(row, col, cell)
            });
            if let Err(error) = result {
                failure = Some(error.into());
                return;
            }
        }
    };
    match part {
        Part::Line(points) => line_spans(points, grid.rows, grid.cols, &mut visit),
        Part::Polygon(rings) => polygon_spans(rings, grid.rows, grid.cols, &mut visit),
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(burnt),
    }
}

/// Burn every feature of `layer` into `cells`, one part at a time.
/// `update_for` gives the update of a feature's cells, or `None` to leave
/// the feature out. A feature that cannot be reprojected, or has
/// non-finite coordinates, fails the burn (it could open a barrier if left
/// out silently).
pub fn burn_layer<T: CellCodec, U: Fn(&mut T)>(
    layer: &VectorLayer,
    grid: &GridSpec,
    cells: &mut TiledGrid<T>,
    burn: Burn,
    cancelled: &dyn Fn() -> bool,
    mut update_for: impl FnMut(&Feature<'_>) -> Result<Option<U>>,
) -> Result<FeatureCounts> {
    let dataset = layer.dataset()?;
    let mut features = layer.layer(&dataset)?;
    let mut counts = FeatureCounts::default();
    for (index, feature) in features.features().enumerate() {
        counts.read += 1;
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let Some(update) = update_for(&feature)? else {
            counts.without_class += 1;
            continue;
        };
        let problem = layer.feature_problem(feature.fid(), index as u64);
        let (mut lines, mut polygons, mut burnt, mut unsupported) = (0u64, 0u64, false, 0u64);
        if let Some(geometry) = feature.geometry() {
            layer.for_each_part(geometry, grid, &problem, &mut unsupported, &mut |part| {
                match part {
                    Part::Polygon(_) if burn == Burn::Lines => return Ok(()),
                    Part::Polygon(_) => polygons += 1,
                    Part::Line(_) => lines += 1,
                }
                burnt |= burn_part(&part, grid, cells, &update, cancelled)?;
                Ok(())
            })?;
        }
        counts.tally(lines, polygons, burnt);
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_attributes() {
        assert_eq!(integer_attribute(FieldValue::IntegerValue(3)), Some(3));
        assert_eq!(integer_attribute(FieldValue::RealValue(4.0)), Some(4));
        assert_eq!(integer_attribute(FieldValue::RealValue(4.5)), None);
        assert_eq!(
            integer_attribute(FieldValue::StringValue(" 12 ".into())),
            Some(12)
        );
        assert_eq!(integer_attribute(FieldValue::StringValue("M".into())), None);
    }
}
