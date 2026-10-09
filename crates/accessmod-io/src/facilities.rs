//! Facility points: reading, explicit reprojection, location on the grid.

use accessmod_core::solver::{Facility, InputCell};
use accessmod_core::store::TiledGrid;
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use gdal::vector::{FieldValue, Geometry, LayerAccess, OGRwkbGeometryType, geometry_type_flatten};
use gdal::{Dataset, DatasetOptions, GdalOpenFlags};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::FacilitySource;

/// A facility used by the solver.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FacilityRecord {
    /// 1-based order among the facilities used (value in
    /// `nearest_facility.tif`).
    pub ordinal: u32,
    /// 0-based position among the features read (after the filter).
    pub feature_index: u64,
    /// Feature id (FID) reported by the driver, if any.
    pub fid: Option<u64>,
    /// Identifier from the `id_field`.
    pub id: serde_json::Value,
    /// Point in the grid CRS.
    pub x: f64,
    /// Point in the grid CRS.
    pub y: f64,
    /// Grid row.
    pub row: usize,
    /// Grid column.
    pub col: usize,
}

/// A facility that cannot be used.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FacilityIssue {
    /// 0-based position among the features read (after the filter).
    pub feature_index: u64,
    /// Feature id (FID) reported by the driver, if any.
    pub fid: Option<u64>,
    /// Identifier, when readable.
    pub id: Option<serde_json::Value>,
    /// What is wrong.
    pub reason: String,
}

/// Facilities read from a layer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoadedFacilities {
    /// Layer read.
    pub layer: String,
    /// Whether points were reprojected to the grid CRS.
    pub reprojected: bool,
    /// Usable facilities, in layer order.
    pub kept: Vec<FacilityRecord>,
    /// Unusable facilities, in layer order.
    pub issues: Vec<FacilityIssue>,
}

impl LoadedFacilities {
    /// Solver facilities, in layer order.
    pub fn solver_facilities(&self) -> Vec<Facility> {
        self.kept
            .iter()
            .map(|f| Facility {
                row: f.row,
                col: f.col,
            })
            .collect()
    }
}

fn traditional(mut srs: SpatialRef) -> SpatialRef {
    srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    srs
}

fn json_id(value: Option<FieldValue>) -> Option<serde_json::Value> {
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

/// Single point of a geometry: a point or a one-point multipoint.
fn single_point(geometry: &Geometry) -> std::result::Result<(f64, f64), String> {
    let kind = geometry_type_flatten(geometry.geometry_type());
    if kind == OGRwkbGeometryType::wkbPoint {
        if geometry.is_empty() {
            return Err("empty point".into());
        }
        let (x, y, _) = geometry.get_point(0);
        return Ok((x, y));
    }
    if kind == OGRwkbGeometryType::wkbMultiPoint && geometry.geometry_count() == 1 {
        let (x, y, _) = geometry.get_geometry(0).get_point(0);
        return Ok((x, y));
    }
    Err(format!(
        "geometry is not a single point ({})",
        gdal::vector::geometry_type_to_name(geometry.geometry_type())
    ))
}

/// Read the facilities, reproject them to the grid CRS when needed and
/// locate them. Facilities outside the grid, on a barrier or without a
/// usable point are returned as issues; the caller decides whether they are
/// errors or warnings.
pub fn load_facilities(
    source: &FacilitySource,
    grid: &GridSpec,
    inputs: &mut TiledGrid<InputCell>,
) -> Result<LoadedFacilities> {
    let path = &source.path;
    let options = DatasetOptions {
        open_flags: GdalOpenFlags::GDAL_OF_VECTOR | GdalOpenFlags::GDAL_OF_READONLY,
        ..DatasetOptions::default()
    };
    let dataset = Dataset::open_ex(path, options).map_err(Error::gdal(path))?;
    let invalid = |issue: String| Error::invalid(format!("facilities {}: {issue}", path.display()));
    let mut layer = match &source.layer {
        Some(name) => dataset
            .layer_by_name(name)
            .map_err(|_| invalid(format!("no layer named {name:?}")))?,
        None => dataset
            .layer(0)
            .map_err(|_| invalid("has no layer".into()))?,
    };
    if let Some(filter) = &source.filter {
        layer
            .set_attribute_filter(filter)
            .map_err(|e| invalid(format!("invalid filter {filter:?} ({e})")))?;
    }
    let layer_name = layer.name();
    let field_index = layer.defn().field_index(&source.id_field).map_err(|_| {
        invalid(format!(
            "layer {layer_name:?} has no field {:?}",
            source.id_field
        ))
    })?;

    let layer_srs = layer
        .spatial_ref()
        .ok_or_else(|| invalid(format!("layer {layer_name:?} has no CRS")))?;
    let reprojected = layer_srs != grid.srs;
    let transform = if reprojected {
        let from = traditional(layer_srs);
        let to = traditional(grid.srs.clone());
        Some(CoordTransform::new(&from, &to).map_err(Error::gdal(path))?)
    } else {
        None
    };

    let mut kept = Vec::new();
    let mut issues = Vec::new();
    for (index, feature) in layer.features().enumerate() {
        let feature_index = index as u64;
        let fid = feature.fid();
        let id = json_id(feature.field(field_index).map_err(Error::gdal(path))?);
        let mut issue = |reason: String| {
            issues.push(FacilityIssue {
                feature_index,
                fid,
                id: id.clone(),
                reason,
            })
        };
        if id.is_none() {
            issue(format!("empty identifier in field {:?}", source.id_field));
            continue;
        }
        let point = match feature.geometry().map(single_point) {
            None => Err("no geometry".to_owned()),
            Some(point) => point,
        };
        let (mut x, mut y) = match point {
            Ok(point) => point,
            Err(reason) => {
                issue(reason);
                continue;
            }
        };
        if let Some(transform) = &transform {
            let (mut xs, mut ys) = ([x], [y]);
            if transform
                .transform_coords(&mut xs, &mut ys, &mut [])
                .is_err()
            {
                issue("cannot be reprojected to the DEM CRS".into());
                continue;
            }
            (x, y) = (xs[0], ys[0]);
        }
        let Some((row, col)) = grid.locate(x, y) else {
            issue("outside the DEM grid".into());
            continue;
        };
        let input = inputs.get(row, col)?;
        if !input.is_passable() {
            let why = if input.dem_m.is_nan() {
                "on a barrier (DEM NoData)"
            } else {
                "on a barrier (land cover NoData or class without scenario)"
            };
            issue(why.into());
            continue;
        }
        let ordinal = u32::try_from(kept.len() + 1)
            .map_err(|_| invalid("more than 4 294 967 295 facilities".into()))?;
        kept.push(FacilityRecord {
            ordinal,
            feature_index,
            fid,
            id: id.expect("checked above"),
            x,
            y,
            row,
            col,
        });
    }
    Ok(LoadedFacilities {
        layer: layer_name,
        reprojected,
        kept,
        issues,
    })
}
