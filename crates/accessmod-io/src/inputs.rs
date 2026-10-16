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
pub const STRIP_ROWS: usize = accessmod_core::store::DEFAULT_TILE;

/// Columns read from GDAL at once, bounding read buffers to
/// `STRIP_ROWS × CHUNK_COLS` cells whatever the grid width.
pub const CHUNK_COLS: usize = 4096;

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
    open_class_raster(path, grid, "land cover")
}

/// Open a raster of integer classes and check it lies on the DEM lattice
/// with the DEM's CRS. `role` names the raster in messages.
pub fn open_class_raster(path: &Path, grid: &GridSpec, role: &str) -> Result<(Dataset, Alignment)> {
    let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
    let prefix = |issues: Vec<String>| Error::Validation {
        issues: issues
            .into_iter()
            .map(|i| format!("{role} {}: {i}", path.display()))
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
pub(crate) fn integer_nodata(value: Option<f64>) -> Option<i64> {
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
    let mut classes = vec![None::<i64>; CHUNK_COLS.min(cols)];
    for start in (0..rows).step_by(STRIP_ROWS) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let height = STRIP_ROWS.min(rows - start);
        // Land-cover rows covering this strip (a contiguous range).
        let covered: Vec<usize> = (start..start + height)
            .filter(|&r| alignment.row_window(r, cols).is_some())
            .collect();
        for chunk in (0..cols).step_by(CHUNK_COLS) {
            let width = CHUNK_COLS.min(cols - chunk);
            let dem_window = dem_band
                .read_as::<f64>(
                    (chunk as isize, start as isize),
                    (width, height),
                    (width, height),
                    None,
                )
                .map_err(Error::gdal(dem_path))?;
            let lc_window = read_land_cover_window(
                &lc_band,
                land_cover_path,
                &alignment,
                &covered,
                chunk,
                width,
                cols,
            )?;
            for r in 0..height {
                let row = start + r;
                let classes = &mut classes[..width];
                classes.fill(None);
                if let Some(window) = &lc_window
                    && let Some(values) = window.row(row)
                {
                    for (k, value) in values.iter().enumerate() {
                        if Some(*value) != lc_nodata {
                            classes[window.first_col - chunk + k] = Some(*value);
                        }
                    }
                }
                for (k, class) in classes.iter().enumerate() {
                    let z = dem_window.data()[r * width + k];
                    let cell = classify(z, dem_nodata, *class, slots, &mut summary);
                    inputs.set(row, chunk + k, cell)?;
                }
            }
        }
    }
    Ok(summary)
}

/// Land-cover values over DEM rows `covered` and DEM columns
/// `first_col .. first_col + width`.
struct LandCoverWindow {
    first_row: usize,
    first_col: usize,
    width: usize,
    values: Vec<i64>,
}

impl LandCoverWindow {
    fn row(&self, row: usize) -> Option<&[i64]> {
        let r = row.checked_sub(self.first_row)?;
        self.values.get(r * self.width..(r + 1) * self.width)
    }
}

/// Read the part of the land cover overlapping DEM rows `covered` and DEM
/// columns `chunk .. chunk + width`, if any.
fn read_land_cover_window(
    band: &gdal::raster::RasterBand<'_>,
    path: &Path,
    alignment: &Alignment,
    covered: &[usize],
    chunk: usize,
    width: usize,
    dem_cols: usize,
) -> Result<Option<LandCoverWindow>> {
    let Some(&first_row) = covered.first() else {
        return Ok(None);
    };
    let Some((raster_col, dem_col, cover_width)) = alignment.row_window(first_row, dem_cols) else {
        return Ok(None);
    };
    let lo = dem_col.max(chunk);
    let hi = (dem_col + cover_width).min(chunk + width);
    if hi <= lo {
        return Ok(None);
    }
    let (window_cols, window_rows) = (hi - lo, covered.len());
    let buffer = band
        .read_as::<i64>(
            (
                (raster_col + lo - dem_col) as isize,
                alignment.raster_row(first_row) as isize,
            ),
            (window_cols, window_rows),
            (window_cols, window_rows),
            None,
        )
        .map_err(Error::gdal(path))?;
    Ok(Some(LandCoverWindow {
        first_row,
        first_col: lo,
        width: window_cols,
        values: buffer.into_shape_and_vec().1,
    }))
}

/// Input cell for DEM value `z` and land-cover class `class`, counted in
/// `summary`.
fn classify(
    z: f64,
    dem_nodata: Option<f64>,
    class: Option<i64>,
    slots: &BTreeMap<i64, u16>,
    summary: &mut InputSummary,
) -> InputCell {
    let dem_valid = z.is_finite() && Some(z) != dem_nodata;
    let slot = match class {
        None => {
            summary.land_cover_nodata += 1;
            InputCell::BARRIER
        }
        Some(value) => match slots.get(&value) {
            Some(&slot) => {
                *summary.class_cells.entry(value).or_default() += 1;
                slot
            }
            None => {
                let tracked = summary.unmapped_classes.len() < MAX_TRACKED_CLASSES
                    || summary.unmapped_classes.contains_key(&value);
                if tracked {
                    *summary.unmapped_classes.entry(value).or_default() += 1;
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
    cell
}
