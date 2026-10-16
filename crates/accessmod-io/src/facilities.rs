//! Facility points: reading, explicit reprojection, location on the grid.

use accessmod_core::solver::{Facility, InputCell};
use accessmod_core::store::TiledGrid;
use gdal::vector::{FieldValue, Geometry, LayerAccess, OGRwkbGeometryType, geometry_type_flatten};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::FacilitySource;
use crate::vector::VectorLayer;

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
    cancelled: &dyn Fn() -> bool,
) -> Result<LoadedFacilities> {
    let vector = VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        "facilities",
    )?;
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let field_index = vector.field_index(&layer, &source.id_field)?;
    let invalid = |issue: String| vector.invalid(issue);
    let path = &source.path;

    let mut kept = Vec::new();
    let mut issues = Vec::new();
    for (index, feature) in layer.features().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
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
        let (mut xs, mut ys) = ([x], [y]);
        if let Err(why) = vector.to_grid_crs(&mut xs, &mut ys) {
            issue(why.into());
            continue;
        }
        (x, y) = (xs[0], ys[0]);
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
        reprojected: vector.reprojected(),
        layer: vector.name,
        kept,
        issues,
    })
}
