//! Vector layers read onto the grid: layer and attribute filter checked up
//! front, explicit reprojection to the DEM CRS, geometries split into line
//! and polygon parts in grid pixel coordinates (see
//! `accessmod_core::raster`).

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use accessmod_core::raster::{line_spans, polygon_coverage, polygon_spans, thin_line_cells};
use accessmod_core::store::{CellCodec, TiledGrid};
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use gdal::vector::{
    Feature, FieldValue, Geometry, Layer, LayerAccess, OGRwkbGeometryType, geometry_type_flatten,
};
use gdal::{Dataset, DatasetOptions, GdalOpenFlags};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::raster::{AlignedBand, valid_value};

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
    selection: Option<Selection>,
    transform: Option<CoordTransform>,
}

/// Identifiers a layer is restricted to, on top of its attribute filter.
/// Applied while reading: OGR SQL cannot parse an `IN` list of thousands
/// of values.
#[derive(Debug)]
struct Selection {
    field: String,
    keys: HashSet<String>,
}

/// A line or polygon of a geometry, in pixel coordinates of the grid.
#[derive(Debug)]
pub enum Part {
    /// Line string or ring.
    Line(Vec<(f64, f64)>),
    /// Exterior ring then holes.
    Polygon(Vec<Vec<(f64, f64)>>),
    /// Point.
    Point((f64, f64)),
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
            selection: None,
            transform,
        })
    }

    /// Restrict the features to those whose `field` value has one of
    /// `keys` ([`id_key`]); read them through [`VectorLayer::features`].
    pub fn select(mut self, field: &str, keys: HashSet<String>) -> Self {
        self.selection = Some(Selection {
            field: field.to_owned(),
            keys,
        });
        self
    }

    /// Features of `layer` (from [`VectorLayer::layer`]) passing the
    /// selection, in layer order.
    pub fn features<'a>(
        &'a self,
        layer: &'a mut Layer<'_>,
    ) -> Result<impl Iterator<Item = Feature<'a>> + 'a> {
        let selection = match &self.selection {
            Some(selection) => Some((self.field_index(layer, &selection.field)?, &selection.keys)),
            None => None,
        };
        Ok(layer.features().filter(move |feature| match selection {
            None => true,
            Some((index, keys)) => feature
                .field(index)
                .ok()
                .and_then(json_attribute)
                .is_some_and(|id| keys.contains(&id_key(&id))),
        }))
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

    /// Values of `fields` for every feature, in layer order (`None`: null).
    pub fn read_fields(
        &self,
        fields: &[&str],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Vec<Option<FieldValue>>>> {
        let dataset = self.dataset()?;
        let mut layer = self.layer(&dataset)?;
        let indices = fields
            .iter()
            .map(|field| self.field_index(&layer, field))
            .collect::<Result<Vec<_>>>()?;
        let mut rows = Vec::new();
        for feature in self.features(&mut layer)? {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            rows.push(
                indices
                    .iter()
                    .map(|&i| feature.field(i).map_err(Error::gdal(&self.path)))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        Ok(rows)
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

    /// `geometry` in the grid CRS (a copy, reprojected when needed), for
    /// geometry operations in map coordinates.
    pub fn to_grid_geometry(
        &self,
        geometry: &Geometry,
    ) -> std::result::Result<Geometry, &'static str> {
        match &self.transform {
            Some(transform) => geometry.transform(transform).map_err(|_| NOT_REPROJECTABLE),
            None => Ok(geometry.clone()),
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

    /// Visit the point, line and polygon parts of `geometry` one at a time,
    /// so that only one part's coordinates are held. Multi-geometries and
    /// collections are split; other parts (curves) are counted in
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
            OGRwkbGeometryType::wkbPoint => {
                let point = self.to_pixels(points(geometry), grid).map_err(problem)?;
                if let Some(&p) = point.first() {
                    visit(Part::Point(p))?;
                }
            }
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
            OGRwkbGeometryType::wkbMultiPoint
            | OGRwkbGeometryType::wkbMultiLineString
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

/// An attribute as a JSON value (identifiers): numbers, text, dates.
pub fn json_attribute(value: Option<FieldValue>) -> Option<serde_json::Value> {
    use serde_json::Value;
    match value? {
        FieldValue::IntegerValue(v) => Some(Value::from(v)),
        FieldValue::Integer64Value(v) => Some(Value::from(v)),
        FieldValue::StringValue(v) => Some(Value::from(v)),
        FieldValue::RealValue(v) => serde_json::Number::from_f64(v).map(Value::Number),
        FieldValue::DateValue(v) => Some(Value::from(v.to_string())),
        FieldValue::DateTimeValue(v) => Some(Value::from(v.to_string())),
        _ => None,
    }
}

/// Identifier as text, to match identifiers whatever the field types
/// (`4`, `4.0` and `"4"` match, as AccessMod 5 compares text).
pub fn id_key(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => match n.as_f64() {
            Some(v) if v.fract() == 0.0 && v.abs() < 1e15 => format!("{}", v as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// Order of attribute values (identifiers): numbers numerically, then text.
pub fn compare_values(a: &serde_json::Value, b: &serde_json::Value) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

/// Numeric value of an attribute: integers, reals and numeric text.
pub fn number_attribute(value: FieldValue) -> Option<f64> {
    match value {
        FieldValue::IntegerValue(v) => Some(f64::from(v)),
        FieldValue::Integer64Value(v) => Some(v as f64),
        FieldValue::RealValue(v) => Some(v),
        FieldValue::StringValue(text) => text.trim().parse().ok(),
        _ => None,
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
    /// Polygons only (as `v.to.rast type=area`); lines are ignored.
    Polygons,
    /// Lines (every cell touched) and polygons (cell centre inside).
    LinesAndPolygons,
    /// Every feature type as `v.to.rast` without `-d`: points (their cell),
    /// thin 8-connected lines and polygons (cell centre inside).
    Thin,
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
    /// Points burnt (`Burn::Thin` only).
    #[serde(skip_serializing_if = "is_zero")]
    pub points: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl FeatureCounts {
    /// Count one feature with `lines` and `polygons` parts.
    fn tally(&mut self, lines: u64, polygons: u64, points: u64, burnt: bool) {
        self.lines += lines;
        self.polygons += polygons;
        self.points += points;
        if lines + polygons + points == 0 {
            self.unsupported_geometry += 1;
        } else if burnt {
            self.used += 1;
        } else {
            self.outside_grid += 1;
        }
    }
}

/// Apply `update` to every cell `part` burns (see `accessmod_core::raster`;
/// `thin`: lines as thin traces);
/// `true` when it burns at least one cell of the grid.
pub fn burn_part<T: CellCodec>(
    part: &Part,
    thin: bool,
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
        Part::Line(points) if thin => thin_line_cells(points, grid.rows, grid.cols, &mut visit),
        Part::Line(points) => line_spans(points, grid.rows, grid.cols, &mut visit),
        Part::Polygon(rings) => polygon_spans(rings, grid.rows, grid.cols, &mut visit),
        Part::Point(point) => thin_line_cells(&[*point], grid.rows, grid.cols, &mut visit),
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
    for (index, feature) in layer.features(&mut features)?.enumerate() {
        counts.read += 1;
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let Some(update) = update_for(&feature)? else {
            counts.without_class += 1;
            continue;
        };
        let problem = layer.feature_problem(feature.fid(), index as u64);
        let (mut lines, mut polygons, mut points, mut burnt, mut unsupported) =
            (0u64, 0u64, 0u64, false, 0u64);
        if let Some(geometry) = feature.geometry() {
            layer.for_each_part(geometry, grid, &problem, &mut unsupported, &mut |part| {
                match part {
                    Part::Polygon(_) if burn == Burn::Lines => return Ok(()),
                    Part::Line(_) if burn == Burn::Polygons => return Ok(()),
                    Part::Point(_) if burn != Burn::Thin => return Ok(()),
                    Part::Polygon(_) => polygons += 1,
                    Part::Line(_) => lines += 1,
                    Part::Point(_) => points += 1,
                }
                burnt |= burn_part(&part, burn == Burn::Thin, grid, cells, &update, cancelled)?;
                Ok(())
            })?;
        }
        counts.tally(lines, polygons, points, burnt);
    }
    Ok(counts)
}

/// Burn zone polygons into a grid of zone indices (0: no zone) by cell
/// centre. `zone_of` gives the index of each feature's zone (`None`: left
/// out); a cell in several zones keeps the first one burnt (layer order).
/// Returns the feature counts and the cells claimed by a later zone.
pub fn burn_zones(
    layer: &VectorLayer,
    grid: &GridSpec,
    zones: &mut TiledGrid<u32>,
    cancelled: &dyn Fn() -> bool,
    mut zone_of: impl FnMut(&Feature<'_>) -> Result<Option<u32>>,
) -> Result<(FeatureCounts, u64)> {
    let overlaps = std::cell::Cell::new(0u64);
    let counts = burn_layer(layer, grid, zones, Burn::Polygons, cancelled, |feature| {
        let overlaps = &overlaps;
        Ok(zone_of(feature)?.map(|zone| {
            move |cell: &mut u32| {
                if *cell == 0 {
                    *cell = zone;
                } else if *cell != zone {
                    overlaps.set(overlaps.get() + 1);
                }
            }
        }))
    })?;
    Ok((counts, overlaps.get()))
}

/// The cells of `dataset` (band 1) whose value is not 0, as one
/// MultiPolygon in its georeferenced coordinates; `None` when no cell is
/// set. Cells sharing an edge form one polygon (GDALPolygonize).
pub fn polygonize(dataset: &Dataset, path: &Path) -> Result<Option<Geometry>> {
    let band = dataset.rasterband(1).map_err(Error::gdal(path))?;
    let driver = gdal::DriverManager::get_driver_by_name("Memory").map_err(Error::gdal(path))?;
    let mut memory = driver.create_vector_only("").map_err(Error::gdal(path))?;
    let layer = memory
        .create_layer(gdal::vector::LayerOptions {
            name: "cells",
            srs: dataset.spatial_ref().ok().as_ref(),
            ty: OGRwkbGeometryType::wkbPolygon,
            options: None,
        })
        .map_err(Error::gdal(path))?;
    layer
        .create_defn_fields(&[("value", gdal::vector::OGRFieldType::OFTInteger)])
        .map_err(Error::gdal(path))?;
    // SAFETY: the band and layer handles stay valid for the call (both
    // owners live until the end of this function); the band is its own mask
    // (0 cells are skipped); no options, no progress callback.
    let status = unsafe {
        gdal_sys::GDALPolygonize(
            band.c_rasterband(),
            band.c_rasterband(),
            layer.c_layer(),
            0,
            std::ptr::null_mut(),
            None,
            std::ptr::null_mut(),
        )
    };
    if status != gdal_sys::CPLErr::CE_None {
        return Err(Error::Gdal {
            path: path.to_path_buf(),
            source: gdal::errors::GdalError::CplError {
                class: status,
                number: 0,
                msg: "GDALPolygonize failed".into(),
            },
        });
    }
    let mut layer = memory.layer(0).map_err(Error::gdal(path))?;
    let mut multi =
        Geometry::empty(OGRwkbGeometryType::wkbMultiPolygon).map_err(Error::gdal(path))?;
    let mut any = false;
    for feature in layer.features() {
        if let Some(polygon) = feature.geometry() {
            multi
                .add_geometry(polygon.clone())
                .map_err(Error::gdal(path))?;
            any = true;
        }
    }
    Ok(any.then_some(multi))
}

/// Visit the polygons of `geometry` (multi-geometries and collections
/// split) as rings of `(x, y)` map coordinates: exterior first.
fn for_each_polygon(geometry: &Geometry, visit: &mut dyn FnMut(Vec<Vec<(f64, f64)>>)) {
    match geometry_type_flatten(geometry.geometry_type()) {
        OGRwkbGeometryType::wkbPolygon => {
            let rings = (0..geometry.geometry_count())
                .map(|i| {
                    let mut points = Vec::new();
                    geometry.get_geometry(i).get_points(&mut points);
                    points.into_iter().map(|(x, y, _)| (x, y)).collect()
                })
                .collect();
            visit(rings);
        }
        OGRwkbGeometryType::wkbMultiPolygon | OGRwkbGeometryType::wkbGeometryCollection => {
            for i in 0..geometry.geometry_count() {
                for_each_polygon(&geometry.get_geometry(i), visit);
            }
        }
        _ => {}
    }
}

/// Sum of `band` over the polygons of `geometry` (grid CRS), each cell
/// weighted by the exact fraction of it they cover, as `exactextractr`
/// `sum` (`accessmod_core::raster::polygon_coverage`); NoData cells are
/// skipped, lines and points cover nothing. Rows are read one at a time;
/// cancellation is checked before each.
pub fn covered_sum(
    geometry: &Geometry,
    grid: &GridSpec,
    band: &AlignedBand<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<f64> {
    let nodata = band.nodata();
    let mut sum = 0.0;
    let mut failure = None;
    for_each_polygon(geometry, &mut |rings| {
        if failure.is_some() {
            return;
        }
        let rings: Vec<Vec<(f64, f64)>> = rings
            .into_iter()
            .map(|ring| ring.into_iter().map(|(x, y)| grid.to_pixel(x, y)).collect())
            .collect();
        polygon_coverage(
            &rings,
            grid.rows,
            grid.cols,
            &mut |row, first, fractions| {
                if failure.is_some() {
                    return ControlFlow::Break(());
                }
                if cancelled() {
                    failure = Some(Error::Cancelled);
                    return ControlFlow::Break(());
                }
                match band.read::<f64>(row..row + 1, first..first + fractions.len()) {
                    Ok(Some(window)) => {
                        for (k, fraction) in fractions.iter().enumerate() {
                            if *fraction > 0.0
                                && let Some(value) = window.get(row, first + k)
                                && let Some(value) = valid_value(value, nodata)
                            {
                                sum += value * fraction;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => failure = Some(e),
                }
                ControlFlow::Continue(())
            },
        );
    });
    match failure {
        Some(e) => Err(e),
        None => Ok(sum),
    }
}

/// Where a geometry lies in a [`GeometrySpool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolSlot {
    offset: u64,
    len: u64,
}

impl SpoolSlot {
    /// Size of the geometry (WKB), bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the geometry has no bytes (never: WKB has a header).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Geometries spooled to a scratch file as WKB, so that memory does not
/// grow with their number or size: written once, read back by slot.
#[derive(Debug)]
pub struct GeometrySpool {
    file: crate::output::ScratchFile,
    handle: std::fs::File,
    end: u64,
    dir: PathBuf,
    tag: String,
}

impl GeometrySpool {
    /// A new, empty spool in `dir`.
    pub fn new(dir: &Path, tag: &str) -> Result<Self> {
        let file = crate::output::ScratchFile::new(dir, tag, "wkb");
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(file.path())
            .map_err(Error::file(file.path()))?;
        Ok(Self {
            file,
            handle,
            end: 0,
            dir: dir.to_path_buf(),
            tag: tag.to_owned(),
        })
    }

    /// Append `geometry`.
    pub fn push(&mut self, geometry: &Geometry) -> Result<SpoolSlot> {
        let wkb = geometry.wkb().map_err(Error::gdal(self.file.path()))?;
        self.push_wkb(&wkb)
    }

    fn push_wkb(&mut self, wkb: &[u8]) -> Result<SpoolSlot> {
        use std::io::{Seek, SeekFrom, Write};
        let path = self.file.path();
        self.handle
            .seek(SeekFrom::Start(self.end))
            .map_err(Error::file(path))?;
        self.handle.write_all(wkb).map_err(Error::file(path))?;
        let slot = SpoolSlot {
            offset: self.end,
            len: wkb.len() as u64,
        };
        self.end += slot.len;
        Ok(slot)
    }

    /// WKB of the geometry at `slot`.
    pub fn wkb(&mut self, slot: SpoolSlot) -> Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let path = self.file.path();
        self.handle
            .seek(SeekFrom::Start(slot.offset))
            .map_err(Error::file(path))?;
        let mut wkb = vec![0; slot.len as usize];
        self.handle
            .read_exact(&mut wkb)
            .map_err(Error::file(path))?;
        Ok(wkb)
    }

    /// The geometry at `slot`.
    pub fn get(&mut self, slot: SpoolSlot) -> Result<Geometry> {
        let wkb = self.wkb(slot)?;
        Geometry::from_wkb(&wkb).map_err(Error::gdal(self.file.path()))
    }

    /// Bytes written so far.
    pub fn bytes(&self) -> u64 {
        self.end
    }

    /// Rewrite the geometries at `slots` (updated in place) into a new
    /// file and drop the old one: the spool then holds only them.
    pub fn compact<'a>(
        &mut self,
        slots: impl IntoIterator<Item = &'a mut SpoolSlot>,
    ) -> Result<()> {
        let mut fresh = Self::new(&self.dir, &self.tag)?;
        for slot in slots {
            let wkb = self.wkb(*slot)?;
            *slot = fresh.push_wkb(&wkb)?;
        }
        *self = fresh;
        Ok(())
    }
}

/// A FlatGeobuf layer written feature by feature (no timestamps: the same
/// features give the same bytes).
#[derive(Debug)]
pub struct FeatureWriter {
    dataset: Dataset,
    path: PathBuf,
}

impl FeatureWriter {
    /// Create `path` with layer `name`, CRS `srs`, geometry type `kind` and
    /// attribute `fields`.
    pub fn create(
        path: &Path,
        name: &str,
        srs: &SpatialRef,
        kind: OGRwkbGeometryType::Type,
        fields: &[(&str, gdal::vector::OGRFieldType::Type)],
    ) -> Result<Self> {
        let driver =
            gdal::DriverManager::get_driver_by_name("FlatGeobuf").map_err(Error::gdal(path))?;
        let mut dataset = driver.create_vector_only(path).map_err(Error::gdal(path))?;
        let layer = dataset
            .create_layer(gdal::vector::LayerOptions {
                name,
                srs: Some(srs),
                ty: kind,
                options: None,
            })
            .map_err(Error::gdal(path))?;
        layer
            .create_defn_fields(fields)
            .map_err(Error::gdal(path))?;
        Ok(Self {
            dataset,
            path: path.to_path_buf(),
        })
    }

    /// Write one feature; `values` follow the fields (`None`: null).
    pub fn write(&mut self, geometry: Geometry, values: &[Option<FieldValue>]) -> Result<()> {
        let layer = self.dataset.layer(0).map_err(Error::gdal(&self.path))?;
        let mut feature = Feature::new(layer.defn()).map_err(Error::gdal(&self.path))?;
        feature
            .set_geometry(geometry)
            .map_err(Error::gdal(&self.path))?;
        for (index, value) in values.iter().enumerate() {
            if let Some(value) = value {
                feature
                    .set_field(index, value)
                    .map_err(Error::gdal(&self.path))?;
            }
        }
        feature.create(&layer).map_err(Error::gdal(&self.path))
    }

    /// Write the index and close the file.
    pub fn close(self) -> Result<()> {
        self.dataset.close().map_err(Error::gdal(&self.path))
    }
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
        assert_eq!(number_attribute(FieldValue::RealValue(2.5)), Some(2.5));
        assert_eq!(
            number_attribute(FieldValue::StringValue("7.25".into())),
            Some(7.25)
        );
        assert_eq!(
            json_attribute(Some(FieldValue::StringValue("HC-1".into()))),
            Some(serde_json::json!("HC-1"))
        );
    }

    #[test]
    fn spool_reads_back_and_compacts() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut spool = GeometrySpool::new(dir.path(), "test").expect("spool");
        let wkt = [
            "POINT (1 2)",
            "POLYGON ((0 0,3 0,3 3,0 3,0 0))",
            "LINESTRING (0 0,5 5,7 1)",
        ];
        let mut slots: Vec<SpoolSlot> = wkt
            .iter()
            .map(|w| {
                spool
                    .push(&Geometry::from_wkt(w).expect("wkt"))
                    .expect("push")
            })
            .collect();
        let as_wkt =
            |spool: &mut GeometrySpool, slot| spool.get(slot).expect("get").wkt().expect("wkt");
        assert_eq!(as_wkt(&mut spool, slots[1]), wkt[1]);
        let before = spool.bytes();
        // Keep the last two only, in reverse order.
        let (_, kept) = slots.split_at_mut(1);
        spool.compact(kept.iter_mut().rev()).expect("compact");
        assert!(spool.bytes() < before);
        assert_eq!(as_wkt(&mut spool, slots[2]), wkt[2]);
        assert_eq!(as_wkt(&mut spool, slots[1]), wkt[1]);
    }
}
