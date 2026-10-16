//! Rasters on the DEM lattice: opening class rasters, and reading any band
//! through its alignment in bounded windows.

use std::ops::Range;
use std::path::Path;

use gdal::Dataset;
use gdal::raster::{GdalType, RasterBand};

use crate::error::{Error, Result};
use crate::grid::{Alignment, GridSpec, RasterFrame, lattice_alignment};

/// Rows read from GDAL at once. Matches the default tile side so a strip
/// fills whole tile rows.
pub const STRIP_ROWS: usize = accessmod_core::store::DEFAULT_TILE;

/// Columns read from GDAL at once, bounding read buffers to
/// `STRIP_ROWS × CHUNK_COLS` cells whatever the grid width.
pub const CHUNK_COLS: usize = 4096;

/// Open a raster of integer classes and check it lies on the DEM lattice
/// with the DEM's CRS. `role` names the raster in messages.
pub fn open_class_raster(path: &Path, grid: &GridSpec, role: &str) -> Result<(Dataset, Alignment)> {
    open_raster(path, grid, role, true)
}

/// Open a raster of any numeric type and check it lies on the DEM lattice
/// with the DEM's CRS. `role` names the raster in messages.
pub fn open_aligned_raster(
    path: &Path,
    grid: &GridSpec,
    role: &str,
) -> Result<(Dataset, Alignment)> {
    open_raster(path, grid, role, false)
}

fn open_raster(
    path: &Path,
    grid: &GridSpec,
    role: &str,
    integer: bool,
) -> Result<(Dataset, Alignment)> {
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
        Ok(band) if integer && !band.band_type().is_integer() => issues.push(format!(
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
pub fn integer_nodata(value: Option<f64>) -> Option<i64> {
    value
        .filter(|v| v.fract() == 0.0 && v.abs() < 9.2e18)
        .map(|v| v as i64)
}

/// `value`, or `None` when it is the band's NoData or NaN.
pub fn valid_value(value: f64, nodata: Option<f64>) -> Option<f64> {
    (Some(value) != nodata && !value.is_nan()).then_some(value)
}

/// Call `visit(rows, cols)` over `rows × cols` in windows of at most
/// `height × width` cells, row of windows by row of windows; cancellation
/// is checked before each row of windows.
pub fn for_each_window(
    rows: Range<usize>,
    cols: Range<usize>,
    (height, width): (usize, usize),
    cancelled: &dyn Fn() -> bool,
    mut visit: impl FnMut(Range<usize>, Range<usize>) -> Result<()>,
) -> Result<()> {
    for start in rows.clone().step_by(height.max(1)) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let strip = start..(start + height).min(rows.end);
        for col in cols.clone().step_by(width.max(1)) {
            visit(strip.clone(), col..(col + width).min(cols.end))?;
        }
    }
    Ok(())
}

/// [`for_each_window`] in read chunks of [`STRIP_ROWS`] × [`CHUNK_COLS`].
pub fn for_each_chunk(
    rows: Range<usize>,
    cols: Range<usize>,
    cancelled: &dyn Fn() -> bool,
    visit: impl FnMut(Range<usize>, Range<usize>) -> Result<()>,
) -> Result<()> {
    for_each_window(rows, cols, (STRIP_ROWS, CHUNK_COLS), cancelled, visit)
}

/// Band 1 of a raster, read in grid coordinates through its alignment on
/// the DEM lattice.
pub struct AlignedBand<'a> {
    band: RasterBand<'a>,
    path: &'a Path,
    alignment: Alignment,
    grid_rows: usize,
    grid_cols: usize,
}

impl std::fmt::Debug for AlignedBand<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlignedBand")
            .field("path", &self.path)
            .field("alignment", &self.alignment)
            .finish_non_exhaustive()
    }
}

/// Values of a raster over a window of grid cells.
#[derive(Debug)]
pub struct Window<T> {
    /// First grid row of the window.
    pub first_row: usize,
    /// First grid column of the window.
    pub first_col: usize,
    /// Columns of the window.
    pub width: usize,
    values: Vec<T>,
}

impl<T> Window<T> {
    /// Values of grid row `row` (from `first_col`), if the window holds it.
    pub fn row(&self, row: usize) -> Option<&[T]> {
        let r = row.checked_sub(self.first_row)?;
        self.values.get(r * self.width..(r + 1) * self.width)
    }

    /// Value of grid cell `(row, col)`, if the window holds it.
    pub fn get(&self, row: usize, col: usize) -> Option<T>
    where
        T: Copy,
    {
        let k = col
            .checked_sub(self.first_col)
            .filter(|k| *k < self.width)?;
        self.row(row).map(|values| values[k])
    }
}

impl<'a> AlignedBand<'a> {
    /// Band 1 of `dataset`, placed on the grid by `alignment`.
    pub fn new(
        dataset: &'a Dataset,
        path: &'a Path,
        alignment: Alignment,
        grid: &GridSpec,
    ) -> Result<Self> {
        Ok(Self {
            band: dataset.rasterband(1).map_err(Error::gdal(path))?,
            path,
            alignment,
            grid_rows: grid.rows,
            grid_cols: grid.cols,
        })
    }

    /// NoData value of the band.
    pub fn nodata(&self) -> Option<f64> {
        self.band.no_data_value()
    }

    /// Grid rows and columns the raster covers.
    pub fn coverage(&self) -> (Range<usize>, Range<usize>) {
        let a = &self.alignment;
        let clip = |offset: i64, len: usize, n: usize| {
            let start = offset.clamp(0, n as i64) as usize;
            let end = (offset + len as i64).clamp(0, n as i64) as usize;
            start..end.max(start)
        };
        (
            clip(a.row_offset, a.rows, self.grid_rows),
            clip(a.col_offset, a.cols, self.grid_cols),
        )
    }

    /// Values over the part of grid `rows × cols` the raster covers, or
    /// `None` when it covers none of it.
    pub fn read<T: GdalType + Copy>(
        &self,
        rows: Range<usize>,
        cols: Range<usize>,
    ) -> Result<Option<Window<T>>> {
        let (covered_rows, covered_cols) = self.coverage();
        let first_row = rows.start.max(covered_rows.start);
        let end_row = rows.end.min(covered_rows.end);
        let first_col = cols.start.max(covered_cols.start);
        let end_col = cols.end.min(covered_cols.end);
        if end_row <= first_row || end_col <= first_col {
            return Ok(None);
        }
        let (width, height) = (end_col - first_col, end_row - first_row);
        let raster_col = first_col as i64 - self.alignment.col_offset;
        let raster_row = first_row as i64 - self.alignment.row_offset;
        let buffer = self
            .band
            .read_as::<T>(
                (raster_col as isize, raster_row as isize),
                (width, height),
                (width, height),
                None,
            )
            .map_err(Error::gdal(self.path))?;
        Ok(Some(Window {
            first_row,
            first_col,
            width,
            values: buffer.into_shape_and_vec().1,
        }))
    }

    /// Call `visit(row, first_col, values)` for every row piece of the grid
    /// the raster covers, read in [`for_each_chunk`] windows.
    pub fn for_each_covered_row<T: GdalType + Copy>(
        &self,
        cancelled: &dyn Fn() -> bool,
        mut visit: impl FnMut(usize, usize, &[T]) -> Result<()>,
    ) -> Result<()> {
        let (rows, cols) = self.coverage();
        for_each_chunk(rows, cols, cancelled, |strip, chunk| {
            if let Some(window) = self.read::<T>(strip.clone(), chunk)? {
                for row in strip {
                    let values = window.row(row).expect("the window holds the strip");
                    visit(row, window.first_col, values)?;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cover_the_range_once() {
        let mut cells = 0;
        for_each_chunk(3..200, 0..9000, &|| false, |rows, cols| {
            assert!(rows.len() <= STRIP_ROWS && cols.len() <= CHUNK_COLS);
            cells += rows.len() * cols.len();
            Ok(())
        })
        .expect("chunks");
        assert_eq!(cells, 197 * 9000);
        let cancelled = for_each_chunk(0..10, 0..10, &|| true, |_, _| Ok(()));
        assert!(matches!(cancelled, Err(Error::Cancelled)));
    }
}
