//! Reading the DEM and land cover into the solver's tiled input grid.

use std::collections::BTreeMap;
use std::path::Path;

use accessmod_core::solver::InputCell;
use accessmod_core::store::TiledGrid;
use gdal::Dataset;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::grid::{Alignment, GridSpec, RasterFrame, lattice_alignment};

/// Rows read from GDAL at once. Matches the default tile side so a strip
/// fills whole tile rows.
const STRIP_ROWS: usize = accessmod_core::store::DEFAULT_TILE;

/// Distinct unmapped classes tracked individually; the rest are counted.
const MAX_TRACKED_CLASSES: usize = 256;

/// Counts gathered while loading inputs, for validation warnings and the
/// run report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct InputSummary {
    /// Cells of the grid.
    pub cells: u64,
    /// Cells that can be entered.
    pub passable: u64,
    /// Cells with DEM NoData (barriers).
    pub dem_nodata: u64,
    /// Cells with land-cover NoData or outside the land-cover raster
    /// (barriers).
    pub land_cover_nodata: u64,
    /// Cells per scenario class.
    pub class_cells: BTreeMap<i64, u64>,
    /// Cells per land-cover class missing from the scenario (barriers).
    pub unmapped_classes: BTreeMap<i64, u64>,
    /// Cells of further unmapped classes beyond the tracked ones.
    pub unmapped_other_cells: u64,
}

/// Open the land cover and check it lies on the DEM lattice with the DEM's
/// CRS and has integer classes.
pub fn open_land_cover(path: &Path, grid: &GridSpec) -> Result<(Dataset, Alignment)> {
    let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
    let prefix = |issues: Vec<String>| Error::Validation {
        issues: issues
            .into_iter()
            .map(|i| format!("land cover {}: {i}", path.display()))
            .collect(),
    };
    let frame = RasterFrame::of(&dataset).map_err(|e| prefix(vec![e]))?;
    let mut issues = frame.check_north_up();
    match dataset.spatial_ref() {
        Ok(srs) if srs == grid.srs => {}
        Ok(_) => issues.push("CRS differs from the DEM's (no reprojection on the fly)".into()),
        Err(_) => issues.push("no coordinate reference system".into()),
    }
    match dataset.rasterband(1) {
        Ok(band) if !band.band_type().is_integer() => issues.push(format!(
            "classes must be integers, band type is {}",
            band.band_type().name()
        )),
        Ok(_) => {}
        Err(e) => issues.push(format!("cannot read band 1 ({e})")),
    }
    if !issues.is_empty() {
        return Err(prefix(issues));
    }
    let alignment = lattice_alignment(&grid.frame(), &frame).map_err(prefix)?;
    Ok((dataset, alignment))
}

/// NoData value of an integer band as an integer, if it has one that is.
fn integer_nodata(value: Option<f64>) -> Option<i64> {
    value
        .filter(|v| v.fract() == 0.0 && v.abs() < 9.2e18)
        .map(|v| v as i64)
}

/// Fill `inputs` from the DEM and the land cover.
///
/// A cell is passable when its DEM value is valid and its land-cover class
/// has a scenario slot; otherwise it is a barrier. `slots` maps class values
/// to scenario slots.
#[allow(clippy::too_many_arguments)] // reason: one call site, explicit inputs read better than a struct
pub fn load_inputs(
    dem: &Dataset,
    dem_path: &Path,
    land_cover: &Dataset,
    land_cover_path: &Path,
    alignment: Alignment,
    grid: &GridSpec,
    slots: &BTreeMap<i64, u16>,
    inputs: &mut TiledGrid<InputCell>,
    cancelled: &dyn Fn() -> bool,
) -> Result<InputSummary> {
    let dem_band = dem.rasterband(1).map_err(Error::gdal(dem_path))?;
    let dem_nodata = dem_band.no_data_value();
    let lc_band = land_cover
        .rasterband(1)
        .map_err(Error::gdal(land_cover_path))?;
    let lc_nodata = integer_nodata(lc_band.no_data_value());
    let (rows, cols) = (grid.rows, grid.cols);

    let mut summary = InputSummary {
        cells: (rows * cols) as u64,
        ..InputSummary::default()
    };
    let mut classes = vec![None::<i64>; cols];
    for start in (0..rows).step_by(STRIP_ROWS) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let height = STRIP_ROWS.min(rows - start);
        let dem_strip = dem_band
            .read_as::<f64>((0, start as isize), (cols, height), (cols, height), None)
            .map_err(Error::gdal(dem_path))?;

        // Land-cover rows covering this strip, read in one window.
        let covered: Vec<usize> = (start..start + height)
            .filter(|&r| alignment.row_window(r, cols).is_some())
            .collect();
        let lc_strip = match (
            covered.first(),
            alignment.row_window(covered.first().copied().unwrap_or(0), cols),
        ) {
            (Some(&first), Some((raster_col, _, width))) => {
                let raster_row = alignment.raster_row(first);
                let window_rows = covered.len();
                let buffer = lc_band
                    .read_as::<i64>(
                        (raster_col as isize, raster_row as isize),
                        (width, window_rows),
                        (width, window_rows),
                        None,
                    )
                    .map_err(Error::gdal(land_cover_path))?;
                Some((first, buffer))
            }
            _ => None,
        };

        for r in 0..height {
            let row = start + r;
            classes.fill(None);
            if let (Some((first, buffer)), Some((_, dem_col, width))) =
                (&lc_strip, alignment.row_window(row, cols))
            {
                let offset = (row - first) * width;
                for (k, value) in buffer.data()[offset..offset + width].iter().enumerate() {
                    if Some(*value) != lc_nodata {
                        classes[dem_col + k] = Some(*value);
                    }
                }
            }
            for (col, class) in classes.iter().enumerate() {
                let z = dem_strip.data()[r * cols + col];
                let dem_valid = z.is_finite() && Some(z) != dem_nodata;
                let slot = match class {
                    None => {
                        summary.land_cover_nodata += 1;
                        InputCell::BARRIER
                    }
                    Some(value) => match slots.get(value) {
                        Some(&slot) => {
                            *summary.class_cells.entry(*value).or_default() += 1;
                            slot
                        }
                        None => {
                            let tracked = summary.unmapped_classes.len() < MAX_TRACKED_CLASSES
                                || summary.unmapped_classes.contains_key(value);
                            if tracked {
                                *summary.unmapped_classes.entry(*value).or_default() += 1;
                            } else {
                                summary.unmapped_other_cells += 1;
                            }
                            InputCell::BARRIER
                        }
                    },
                };
                if !dem_valid {
                    summary.dem_nodata += 1;
                }
                let cell = InputCell {
                    slot,
                    dem_m: if dem_valid { z } else { f64::NAN },
                };
                if cell.is_passable() {
                    summary.passable += 1;
                }
                inputs.set(row, col, cell)?;
            }
        }
    }
    Ok(summary)
}
