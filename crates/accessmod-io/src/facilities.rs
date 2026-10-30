//! Facility points: reading, explicit reprojection, location on the grid.

use std::collections::{BTreeSet, HashSet};

use accessmod_core::solver::{Facility, InputCell};
use accessmod_core::store::TiledGrid;
use gdal::vector::{Geometry, LayerAccess, OGRwkbGeometryType, geometry_type_flatten};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::job::{FacilityId, FacilitySource};
use crate::vector::{VectorLayer, id_key, json_attribute};

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

/// Single point of a geometry: a point or a one-point multipoint.
pub fn single_point(geometry: &Geometry) -> std::result::Result<(f64, f64), String> {
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

/// Open a facility layer with its attribute filter (`where`) and its
/// `ids` selection. Every selected identifier must name a feature matching
/// `where`: a selection saved from a table and replayed on another layer
/// fails instead of silently running on fewer facilities.
pub fn open_facilities(
    source: &FacilitySource,
    grid: &GridSpec,
    role: &str,
) -> Result<VectorLayer> {
    let vector = VectorLayer::open(
        &source.path,
        source.layer.as_deref(),
        source.filter.as_deref(),
        grid,
        role,
    )?;
    let Some(ids) = &source.ids else {
        return Ok(vector);
    };
    if ids.is_empty() {
        return Err(vector.invalid("ids selects no facility (omit it to use all)".into()));
    }
    let keys: HashSet<String> = ids.iter().map(FacilityId::key).collect();
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let field_index = vector.field_index(&layer, &source.id_field)?;
    let mut missing: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
    for feature in layer.features() {
        let id = json_attribute(
            feature
                .field(field_index)
                .map_err(Error::gdal(&source.path))?,
        );
        if let Some(id) = id {
            missing.remove(id_key(&id).as_str());
        }
        if missing.is_empty() {
            break;
        }
    }
    if !missing.is_empty() {
        let shown: Vec<&str> = missing.iter().take(10).copied().collect();
        let more = missing.len() - shown.len();
        let tail = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        let filter = source.filter.as_deref().map(|f| format!(" where {f}"));
        return Err(vector.invalid(format!(
            "ids not found in {:?}{}: {}{tail}",
            source.id_field,
            filter.unwrap_or_default(),
            shown.join(", ")
        )));
    }
    Ok(vector.select(&source.id_field, keys))
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
    let vector = open_facilities(source, grid, "facilities")?;
    let dataset = vector.dataset()?;
    let mut layer = vector.layer(&dataset)?;
    let field_index = vector.field_index(&layer, &source.id_field)?;
    let invalid = |issue: String| vector.invalid(issue);
    let path = &source.path;

    let mut kept = Vec::new();
    let mut issues = Vec::new();
    for (index, feature) in vector.features(&mut layer)?.enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let feature_index = index as u64;
        let fid = feature.fid();
        let id = json_attribute(feature.field(field_index).map_err(Error::gdal(path))?);
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
