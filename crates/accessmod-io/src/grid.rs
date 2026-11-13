//! The DEM grid and the lattice rule for every other raster.
//!
//! The DEM is the absolute reference (`docs/plan.md`, "Travel-time job v1"):
//! it defines CRS, resolution, origin and extent. Another raster is accepted
//! only if it lies on the DEM lattice; it is then read through a window and
//! cells it does not cover are NoData. There is no resampling or reprojection.

use std::path::Path;

use accessmod_core::cost::CellSize;
use gdal::Dataset;
use gdal::spatial_ref::SpatialRef;

use crate::error::Error;

/// Maximum relative difference between two resolutions.
pub const RESOLUTION_TOLERANCE: f64 = 1e-9;
/// Maximum distance, in cells, between an origin offset and a whole number of
/// cells.
pub const ORIGIN_TOLERANCE_CELLS: f64 = 1e-6;
/// Maximum rotation term of a geotransform, relative to the cell size.
pub const ROTATION_TOLERANCE: f64 = 1e-12;

/// Grid of the DEM: size, north-up geotransform and projected metric CRS.
#[derive(Debug, Clone)]
pub struct GridSpec {
    /// Columns.
    pub cols: usize,
    /// Rows.
    pub rows: usize,
    /// GDAL geotransform `[x0, ew, 0, y0, 0, -ns]`.
    pub geo_transform: [f64; 6],
    /// CRS of the grid.
    pub srs: SpatialRef,
    /// Cell size.
    pub cell: CellSize,
}

/// Bounds in a CRS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    /// West.
    pub xmin: f64,
    /// South.
    pub ymin: f64,
    /// East.
    pub xmax: f64,
    /// North.
    pub ymax: f64,
}

/// Size and geotransform of a raster, independent of GDAL objects.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterFrame {
    /// Columns.
    pub cols: usize,
    /// Rows.
    pub rows: usize,
    /// GDAL geotransform.
    pub geo_transform: [f64; 6],
}

impl RasterFrame {
    /// Frame of an open dataset.
    pub fn of(dataset: &Dataset) -> Result<Self, String> {
        let (cols, rows) = dataset.raster_size();
        let geo_transform = dataset
            .geo_transform()
            .map_err(|e| format!("no geotransform ({e})"))?;
        Ok(Self {
            cols,
            rows,
            geo_transform,
        })
    }

    /// Problems with the geotransform itself: rotation, non north-up,
    /// non-positive or non-finite resolution.
    pub fn check_north_up(&self) -> Vec<String> {
        let [x0, ew, rx, y0, ry, ns] = self.geo_transform;
        let mut issues = Vec::new();
        if ![x0, ew, rx, y0, ry, ns].iter().all(|v| v.is_finite()) {
            issues.push("geotransform has non-finite terms".to_owned());
            return issues;
        }
        if !(ew > 0.0 && ns < 0.0) {
            issues.push(format!(
                "grid must be north-up with positive cell size (pixel width {ew}, height {ns})"
            ));
        }
        let scale = ew.abs().max(ns.abs());
        if rx.abs() > ROTATION_TOLERANCE * scale || ry.abs() > ROTATION_TOLERANCE * scale {
            issues.push(format!(
                "rotated grids are not supported (rotation terms {rx}, {ry})"
            ));
        }
        if self.cols == 0 || self.rows == 0 {
            issues.push("raster is empty".to_owned());
        }
        issues
    }
}

/// Position of a raster on the DEM lattice: its cell `(0, 0)` is DEM cell
/// `(row_offset, col_offset)` (offsets may be negative when it starts
/// outside the DEM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alignment {
    /// Row of the raster's first row in DEM rows.
    pub row_offset: i64,
    /// Column of the raster's first column in DEM columns.
    pub col_offset: i64,
    /// Raster rows.
    pub rows: usize,
    /// Raster columns.
    pub cols: usize,
}

impl Alignment {
    /// For DEM row `row`, the raster window `(raster_col_start, dem_col_start,
    /// width)` covering it, or `None` when the raster does not cover that row.
    pub fn row_window(&self, row: usize, dem_cols: usize) -> Option<(usize, usize, usize)> {
        let raster_row = row as i64 - self.row_offset;
        if raster_row < 0 || raster_row >= self.rows as i64 {
            return None;
        }
        let dem_start = self.col_offset.max(0);
        let dem_end = (self.col_offset + self.cols as i64).min(dem_cols as i64);
        (dem_end > dem_start).then(|| {
            (
                (dem_start - self.col_offset) as usize,
                dem_start as usize,
                (dem_end - dem_start) as usize,
            )
        })
    }

    /// Raster row for DEM row `row` (callers check coverage first).
    pub fn raster_row(&self, row: usize) -> usize {
        (row as i64 - self.row_offset) as usize
    }
}

/// Position of `other` on the lattice of `dem`, or the reasons it is not on
/// it. Both frames must be north-up.
pub fn lattice_alignment(dem: &RasterFrame, other: &RasterFrame) -> Result<Alignment, Vec<String>> {
    let [dx0, dew, _, dy0, _, dns] = dem.geo_transform;
    let [ox0, oew, _, oy0, _, ons] = other.geo_transform;
    let mut issues = Vec::new();
    let close = |a: f64, b: f64| (a - b).abs() <= RESOLUTION_TOLERANCE * a.abs().max(b.abs());
    if !close(dew, oew) || !close(dns, ons) {
        issues.push(format!(
            "resolution {} x {} differs from the DEM's {} x {}",
            oew,
            ons.abs(),
            dew,
            dns.abs()
        ));
    }
    let col_shift = (ox0 - dx0) / dew;
    let row_shift = (oy0 - dy0) / dns;
    let col_offset = col_shift.round();
    let row_offset = row_shift.round();
    if (col_shift - col_offset).abs() > ORIGIN_TOLERANCE_CELLS
        || (row_shift - row_offset).abs() > ORIGIN_TOLERANCE_CELLS
    {
        issues.push(format!(
            "origin is not on the DEM lattice: offset of {col_shift:.6} columns and \
             {row_shift:.6} rows from the DEM origin (must be whole cells within \
             {ORIGIN_TOLERANCE_CELLS} cell)"
        ));
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    let alignment = Alignment {
        row_offset: row_offset as i64,
        col_offset: col_offset as i64,
        rows: other.rows,
        cols: other.cols,
    };
    let overlaps = alignment.row_offset < dem.rows as i64
        && alignment.row_offset + other.rows as i64 > 0
        && alignment.col_offset < dem.cols as i64
        && alignment.col_offset + other.cols as i64 > 0;
    if !overlaps {
        return Err(vec!["raster does not overlap the DEM".to_owned()]);
    }
    Ok(alignment)
}

/// CRS problems: missing, geographic, or with non-metre linear units.
pub fn check_projected_metric(srs: Option<&SpatialRef>) -> Vec<String> {
    let Some(srs) = srs else {
        return vec!["no coordinate reference system".to_owned()];
    };
    let mut issues = Vec::new();
    if srs.is_geographic() || !srs.is_projected() {
        issues.push("CRS must be projected (geographic CRS are not supported in v1)".to_owned());
    } else if (srs.linear_units() - 1.0).abs() > 1e-12 {
        issues.push(format!(
            "CRS linear unit must be the metre, got {} ({} m)",
            srs.linear_units_name()
                .unwrap_or_else(|| "unknown".to_owned()),
            srs.linear_units()
        ));
    }
    issues
}

impl GridSpec {
    /// Open the DEM at `path` and the grid it defines.
    pub fn open_dem(path: &Path) -> crate::error::Result<(Dataset, Self)> {
        let dem = Dataset::open(path).map_err(Error::gdal(path))?;
        let grid = Self::from_dem(&dem, path).map_err(|issues| Error::Validation { issues })?;
        Ok((dem, grid))
    }

    /// Grid of the DEM at `path`, or every reason it cannot define a grid.
    pub fn from_dem(dataset: &Dataset, path: &Path) -> Result<Self, Vec<String>> {
        Self::from_raster(dataset, path, "DEM")
    }

    /// Open the raster at `path` that defines the grid of an analysis
    /// without a DEM (`role` names it in messages).
    pub fn open_raster(path: &Path, role: &str) -> crate::error::Result<(Dataset, Self)> {
        let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
        let grid = Self::from_raster(&dataset, path, role)
            .map_err(|issues| Error::Validation { issues })?;
        Ok((dataset, grid))
    }

    /// Grid of the raster at `path`, or every reason it cannot define one.
    fn from_raster(dataset: &Dataset, path: &Path, role: &str) -> Result<Self, Vec<String>> {
        let name = path.display();
        let prefix = |issues: Vec<String>| -> Vec<String> {
            issues
                .into_iter()
                .map(|i| format!("{role} {name}: {i}"))
                .collect()
        };
        let frame = RasterFrame::of(dataset).map_err(|e| prefix(vec![e]))?;
        let mut issues = frame.check_north_up();
        let srs = dataset.spatial_ref().ok();
        issues.extend(check_projected_metric(srs.as_ref()));
        if dataset.raster_count() < 1 {
            issues.push("has no band".to_owned());
        }
        let cell = CellSize::new(frame.geo_transform[1], -frame.geo_transform[5]);
        match (issues.is_empty(), srs, cell) {
            (true, Some(srs), Some(cell)) => Ok(Self {
                cols: frame.cols,
                rows: frame.rows,
                geo_transform: frame.geo_transform,
                srs,
                cell,
            }),
            _ => Err(prefix(issues)),
        }
    }

    /// The grid covering `extent` (grid CRS coordinates) with square cells of
    /// `resolution` metres in `srs`, its corners on the lattice through
    /// `anchor` (any cell corner of the lattice: `(0, 0)`, or the origin of a
    /// reference raster, so the rasters of both line up). The extent is
    /// widened to whole cells; at most `max_cells` cells.
    pub fn from_extent(
        extent: Extent,
        resolution: f64,
        srs: SpatialRef,
        anchor: (f64, f64),
        max_cells: u64,
    ) -> Result<Self, Vec<String>> {
        let mut issues = check_projected_metric(Some(&srs));
        let finite = [
            extent.xmin,
            extent.ymin,
            extent.xmax,
            extent.ymax,
            anchor.0,
            anchor.1,
        ]
        .iter()
        .all(|v| v.is_finite());
        if !finite || extent.xmin >= extent.xmax || extent.ymin >= extent.ymax {
            issues.push("the extent is empty or not finite".to_owned());
        }
        if !(resolution.is_finite() && resolution > 0.0) {
            issues.push(format!(
                "resolution {resolution} must be a positive number of metres"
            ));
        }
        if !issues.is_empty() {
            return Err(issues);
        }
        // Whole cells from the anchor, outwards.
        let snap_down = |v: f64, a: f64| a + ((v - a) / resolution).floor() * resolution;
        let snap_up = |v: f64, a: f64| a + ((v - a) / resolution).ceil() * resolution;
        let (x0, x1) = (
            snap_down(extent.xmin, anchor.0),
            snap_up(extent.xmax, anchor.0),
        );
        let (y0, y1) = (
            snap_down(extent.ymin, anchor.1),
            snap_up(extent.ymax, anchor.1),
        );
        let cols = ((x1 - x0) / resolution).round();
        let rows = ((y1 - y0) / resolution).round();
        let cells = cols * rows;
        if !(cells >= 1.0 && cells <= max_cells as f64) {
            return Err(vec![format!(
                "{cols} × {rows} cells at {resolution} m: more than {max_cells}; \
                 choose a coarser resolution or a smaller region"
            )]);
        }
        let cell = CellSize::new(resolution, resolution)
            .ok_or_else(|| vec![format!("resolution {resolution} is not a cell size")])?;
        Ok(Self {
            cols: cols as usize,
            rows: rows as usize,
            geo_transform: [x0, resolution, 0.0, y1, 0.0, -resolution],
            srs,
            cell,
        })
    }

    /// The outline of the grid, a polygon in its CRS.
    pub fn outline(&self) -> crate::error::Result<gdal::vector::Geometry> {
        let (x0, y0) = self.to_map(0.0, 0.0);
        let (x1, y1) = self.to_map(self.cols as f64, self.rows as f64);
        gdal::vector::Geometry::from_wkt(&format!(
            "POLYGON (({x0} {y0}, {x1} {y0}, {x1} {y1}, {x0} {y1}, {x0} {y0}))"
        ))
        .map_err(Error::gdal("grid outline"))
    }

    /// The sub-grid of cells `rows × cols` (same CRS and cell size).
    pub fn window(&self, rows: std::ops::Range<usize>, cols: std::ops::Range<usize>) -> Self {
        let (x0, y0) = self.to_map(cols.start as f64, rows.start as f64);
        let mut geo_transform = self.geo_transform;
        geo_transform[0] = x0;
        geo_transform[3] = y0;
        Self {
            cols: cols.len(),
            rows: rows.len(),
            geo_transform,
            srs: self.srs.clone(),
            cell: self.cell,
        }
    }

    /// Frame of the grid.
    pub fn frame(&self) -> RasterFrame {
        RasterFrame {
            cols: self.cols,
            rows: self.rows,
            geo_transform: self.geo_transform,
        }
    }

    /// Alignment of the grid on itself (the DEM's own raster).
    pub fn alignment(&self) -> Alignment {
        Alignment {
            row_offset: 0,
            col_offset: 0,
            rows: self.rows,
            cols: self.cols,
        }
    }

    /// Pixel coordinates `(column, row)` of point `(x, y)` in grid CRS
    /// coordinates, fractional: cell `(r, c)` covers `[c, c + 1) × [r, r + 1)`.
    pub fn to_pixel(&self, x: f64, y: f64) -> (f64, f64) {
        let [x0, ew, _, y0, _, ns] = self.geo_transform;
        ((x - x0) / ew, (y - y0) / ns)
    }

    /// Cell containing point `(x, y)` in grid CRS coordinates, or `None`
    /// outside the grid. A point on a cell edge belongs to the cell east of
    /// it and south of it (GDAL pixel convention: `floor`).
    pub fn locate(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let (col, row) = self.to_pixel(x, y);
        let (col, row) = (col.floor(), row.floor());
        let inside = col >= 0.0 && row >= 0.0 && col < self.cols as f64 && row < self.rows as f64;
        inside.then_some((row as usize, col as usize))
    }

    /// Centre of cell `(row, col)` in grid CRS coordinates.
    pub fn cell_centre(&self, row: usize, col: usize) -> (f64, f64) {
        self.to_map(col as f64 + 0.5, row as f64 + 0.5)
    }

    /// Grid CRS coordinates of pixel coordinates `(column, row)`, the
    /// inverse of [`GridSpec::to_pixel`].
    pub fn to_map(&self, col: f64, row: f64) -> (f64, f64) {
        let [x0, ew, _, y0, _, ns] = self.geo_transform;
        (x0 + col * ew, y0 + row * ns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(cols: usize, rows: usize, x0: f64, y0: f64, res: f64) -> RasterFrame {
        RasterFrame {
            cols,
            rows,
            geo_transform: [x0, res, 0.0, y0, 0.0, -res],
        }
    }

    #[test]
    fn identical_grids_align_at_zero() {
        let dem = frame(10, 8, 1000.0, 5000.0, 100.0);
        let a = lattice_alignment(&dem, &dem).expect("aligned");
        assert_eq!((a.row_offset, a.col_offset), (0, 0));
        assert_eq!(a.row_window(3, 10), Some((0, 0, 10)));
    }

    #[test]
    fn larger_raster_on_the_lattice_is_windowed() {
        let dem = frame(10, 8, 1000.0, 5000.0, 100.0);
        // Starts 2 columns west and 3 rows north of the DEM, much larger.
        let pop = frame(40, 40, 800.0, 5300.0, 100.0);
        let a = lattice_alignment(&dem, &pop).expect("aligned");
        assert_eq!((a.row_offset, a.col_offset), (-3, -2));
        assert_eq!(a.row_window(0, 10), Some((2, 0, 10)));
        assert_eq!(a.raster_row(0), 3);
    }

    #[test]
    fn smaller_raster_leaves_uncovered_cells() {
        let dem = frame(10, 8, 1000.0, 5000.0, 100.0);
        let lc = frame(4, 2, 1300.0, 4800.0, 100.0);
        let a = lattice_alignment(&dem, &lc).expect("aligned");
        assert_eq!((a.row_offset, a.col_offset), (2, 3));
        assert_eq!(a.row_window(1, 10), None);
        assert_eq!(a.row_window(2, 10), Some((0, 3, 4)));
        assert_eq!(a.row_window(4, 10), None);
    }

    #[test]
    fn tiny_float_noise_is_tolerated() {
        let dem = frame(10, 8, 1000.0, 5000.0, 100.0);
        let lc = frame(10, 8, 1000.0 + 1e-6, 5000.0, 100.0 * (1.0 + 1e-12));
        assert!(lattice_alignment(&dem, &lc).is_ok());
    }

    #[test]
    fn misaligned_rasters_are_rejected() {
        let dem = frame(10, 8, 1000.0, 5000.0, 100.0);
        let shifted = frame(10, 8, 1050.0, 5000.0, 100.0);
        let issues = lattice_alignment(&dem, &shifted).expect_err("half-cell shift");
        assert!(issues[0].contains("lattice"), "{issues:?}");
        let coarser = frame(10, 8, 1000.0, 5000.0, 200.0);
        assert!(lattice_alignment(&dem, &coarser).is_err());
        let far = frame(10, 8, 1e6, 5000.0, 100.0);
        let issues = lattice_alignment(&dem, &far).expect_err("no overlap");
        assert!(issues[0].contains("overlap"));
    }

    #[test]
    fn north_up_check() {
        assert!(frame(2, 2, 0.0, 0.0, 1.0).check_north_up().is_empty());
        let mut rotated = frame(2, 2, 0.0, 0.0, 1.0);
        rotated.geo_transform[2] = 0.1;
        assert_eq!(rotated.check_north_up().len(), 1);
        let mut south_up = frame(2, 2, 0.0, 0.0, 1.0);
        south_up.geo_transform[5] = 1.0;
        assert_eq!(south_up.check_north_up().len(), 1);
    }

    fn utm36() -> SpatialRef {
        SpatialRef::from_epsg(32636).expect("srs")
    }

    #[test]
    fn grids_from_extents_snap_to_the_lattice() {
        let extent = Extent {
            xmin: 555_012.0,
            ymin: 995_050.0,
            xmax: 609_990.0,
            ymax: 1_049_901.0,
        };
        let grid =
            GridSpec::from_extent(extent, 100.0, utm36(), (0.0, 0.0), u64::MAX).expect("grid");
        assert_eq!(
            grid.geo_transform,
            [555_000.0, 100.0, 0.0, 1_050_000.0, 0.0, -100.0]
        );
        assert_eq!((grid.cols, grid.rows), (550, 550));
        // On the lattice of a reference raster (origin 12.5 m off).
        let grid =
            GridSpec::from_extent(extent, 100.0, utm36(), (12.5, -7.5), u64::MAX).expect("grid");
        assert_eq!(grid.geo_transform[0], 554_912.5);
        assert_eq!(grid.geo_transform[3], 1_049_992.5);
        assert_eq!((grid.cols, grid.rows), (551, 550));
        // The extent lies inside the grid.
        let (x1, y1) = grid.to_map(grid.cols as f64, grid.rows as f64);
        assert!(grid.geo_transform[0] <= extent.xmin && x1 >= extent.xmax);
        assert!(y1 <= extent.ymin && grid.geo_transform[3] >= extent.ymax);
    }

    #[test]
    fn grids_from_extents_refuse_bad_input() {
        let ok = Extent {
            xmin: 0.0,
            ymin: 0.0,
            xmax: 1000.0,
            ymax: 1000.0,
        };
        let wgs84 = SpatialRef::from_epsg(4326).expect("srs");
        assert!(GridSpec::from_extent(ok, 100.0, wgs84, (0.0, 0.0), u64::MAX).is_err());
        for resolution in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(GridSpec::from_extent(ok, resolution, utm36(), (0.0, 0.0), u64::MAX).is_err());
        }
        let empty = Extent { xmax: 0.0, ..ok };
        assert!(GridSpec::from_extent(empty, 100.0, utm36(), (0.0, 0.0), u64::MAX).is_err());
        let nan = Extent {
            xmin: f64::NAN,
            ..ok
        };
        assert!(GridSpec::from_extent(nan, 100.0, utm36(), (0.0, 0.0), u64::MAX).is_err());
        let error = GridSpec::from_extent(ok, 100.0, utm36(), (0.0, 0.0), 99).unwrap_err();
        assert!(error[0].contains("coarser"), "{error:?}");
        assert!(GridSpec::from_extent(ok, 1e-300, utm36(), (0.0, 0.0), u64::MAX).is_err());
    }
}
