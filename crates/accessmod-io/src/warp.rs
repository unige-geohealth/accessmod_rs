//! Rasters from elsewhere onto a project grid: reprojection and resampling
//! of one or several source tiles, chunk by chunk under a memory bound
//! (`docs/project-creator.md`, contract 5).
//!
//! The shipped GDAL opens no VRT, and warping tiles one after another into a
//! grid shows their seams (a bilinear or mode kernel near a tile edge misses
//! the neighbouring tile). So each output chunk gets its own mosaic: the
//! source window under the chunk, plus a margin for the kernel, copied from
//! every tile that intersects it into one in-memory raster, then a single
//! `GDALWarp` from that mosaic to the chunk. Tiles must share a CRS and a
//! lattice (one product: COG tiles of a DEM, of a land cover).
//!
//! Source overviews are never read: the mosaic is built from full
//! resolution windows.

use std::ffi::{CString, c_char, c_int};
use std::path::{Path, PathBuf};

use gdal::raster::{Buffer, GdalType};
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use gdal::{Dataset, DriverManager};

use crate::analysis::Cancelled;
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::output::GeoTiff;
use crate::raster::for_each_window;

/// How cells are resampled onto the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resample {
    /// Nearest cell (identifiers).
    Nearest,
    /// Most frequent value (classes: land cover).
    Mode,
    /// Bilinear interpolation (continuous: elevation).
    Bilinear,
    /// Sum of the covered source cells, weighted by overlap (counts:
    /// population).
    Sum,
}

impl Resample {
    fn gdal(self) -> &'static str {
        match self {
            Resample::Nearest => "near",
            Resample::Mode => "mode",
            Resample::Bilinear => "bilinear",
            Resample::Sum => "sum",
        }
    }
}

/// Source cells a kernel may read beyond the cells under a chunk.
const MARGIN: usize = 2;
/// Points per edge of a chunk projected into the source CRS.
const EDGE_SAMPLES: usize = 9;

/// A source tile, open.
struct Tile {
    path: PathBuf,
    dataset: Dataset,
    /// Offset of the tile on the common lattice, in cells.
    col: i64,
    row: i64,
    cols: usize,
    rows: usize,
    nodata: Option<f64>,
}

/// The tiles of one product and their common lattice.
struct Mosaic {
    tiles: Vec<Tile>,
    srs: SpatialRef,
    /// Geotransform of the lattice (origin at the first tile).
    geo_transform: [f64; 6],
    /// Extent of all tiles on the lattice, in cells: columns and rows.
    cols: std::ops::Range<i64>,
    rows: std::ops::Range<i64>,
}

fn gdal_error(path: &Path, message: &str) -> Error {
    Error::Gdal {
        path: path.to_path_buf(),
        source: gdal::errors::GdalError::CplError {
            class: gdal_sys::CPLErr::CE_Failure,
            number: 0,
            msg: message.to_owned(),
        },
    }
}

impl Mosaic {
    fn open(paths: &[PathBuf]) -> Result<Self> {
        let first = paths
            .first()
            .ok_or_else(|| Error::invalid("no source raster to warp"))?;
        let mut tiles = Vec::with_capacity(paths.len());
        let mut lattice: Option<([f64; 6], SpatialRef)> = None;
        for path in paths {
            let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
            let gt = dataset.geo_transform().map_err(Error::gdal(path))?;
            let srs = dataset.spatial_ref().map_err(Error::gdal(path))?;
            if gt[2] != 0.0 || gt[4] != 0.0 || gt[1] <= 0.0 || gt[5] >= 0.0 {
                return Err(Error::invalid(format!(
                    "{}: not a north-up raster",
                    path.display()
                )));
            }
            let (cols, rows) = dataset.raster_size();
            let nodata = dataset
                .rasterband(1)
                .map_err(Error::gdal(path))?
                .no_data_value();
            let (base, base_srs) = lattice.get_or_insert_with(|| (gt, srs.clone()));
            let same_cell = ((gt[1] - base[1]) / base[1]).abs() < 1e-9
                && ((gt[5] - base[5]) / base[5]).abs() < 1e-9;
            let col = (gt[0] - base[0]) / base[1];
            let row = (gt[3] - base[3]) / base[5];
            let on_lattice = (col - col.round()).abs() < 1e-6 && (row - row.round()).abs() < 1e-6;
            if *base_srs != srs || !same_cell || !on_lattice {
                return Err(Error::invalid(format!(
                    "{}: not on the CRS and lattice of {}",
                    path.display(),
                    first.display()
                )));
            }
            tiles.push(Tile {
                path: path.clone(),
                dataset,
                col: col.round() as i64,
                row: row.round() as i64,
                cols,
                rows,
                nodata,
            });
        }
        let (geo_transform, srs) = lattice.ok_or_else(|| Error::invalid("no source raster"))?;
        let cols = tiles.iter().map(|t| t.col).min().unwrap_or(0)
            ..tiles
                .iter()
                .map(|t| t.col + t.cols as i64)
                .max()
                .unwrap_or(0);
        let rows = tiles.iter().map(|t| t.row).min().unwrap_or(0)
            ..tiles
                .iter()
                .map(|t| t.row + t.rows as i64)
                .max()
                .unwrap_or(0);
        Ok(Self {
            tiles,
            srs,
            geo_transform,
            cols,
            rows,
        })
    }

    /// Source cells per grid cell, along x and y, through the centre of the
    /// grid: one value for the whole grid, so every chunk resamples with the
    /// same kernel.
    fn cells_per_cell(&self, grid: &GridSpec, to_source: &CoordTransform) -> Result<(f64, f64)> {
        let (x, y) = grid.to_map(grid.cols as f64 / 2.0, grid.rows as f64 / 2.0);
        let mut xs = [x, x + grid.geo_transform[1], x];
        let mut ys = [y, y, y + grid.geo_transform[5]];
        to_source
            .transform_coords(&mut xs, &mut ys, &mut [])
            .map_err(|_| Error::invalid("the grid does not project into the source CRS"))?;
        let along_x = (xs[1] - xs[0]).hypot(ys[1] - ys[0]) / self.geo_transform[1];
        let along_y = (xs[2] - xs[0]).hypot(ys[2] - ys[0]) / -self.geo_transform[5];
        if !(along_x.is_finite() && along_y.is_finite() && along_x > 0.0 && along_y > 0.0) {
            return Err(Error::invalid(
                "the grid does not project into the source CRS",
            ));
        }
        Ok((along_x, along_y))
    }

    /// The window of the lattice, in cells, under the grid window `frame`
    /// (plus the kernel margin), clipped to the tiles; `None` when nothing
    /// is under it.
    fn window_under(
        &self,
        frame: &GridSpec,
        to_source: &CoordTransform,
    ) -> Option<(std::ops::Range<i64>, std::ops::Range<i64>)> {
        let mut xs = Vec::with_capacity(EDGE_SAMPLES * 4);
        let mut ys = Vec::with_capacity(EDGE_SAMPLES * 4);
        for i in 0..EDGE_SAMPLES {
            let t = i as f64 / (EDGE_SAMPLES - 1) as f64;
            for (c, r) in [
                (t * frame.cols as f64, 0.0),
                (t * frame.cols as f64, frame.rows as f64),
                (0.0, t * frame.rows as f64),
                (frame.cols as f64, t * frame.rows as f64),
            ] {
                let (x, y) = frame.to_map(c, r);
                xs.push(x);
                ys.push(y);
            }
        }
        to_source.transform_coords(&mut xs, &mut ys, &mut []).ok()?;
        let [x0, ew, _, y0, _, ns] = self.geo_transform;
        let cols: Vec<f64> = xs.iter().map(|x| (x - x0) / ew).collect();
        let rows: Vec<f64> = ys.iter().map(|y| (y - y0) / ns).collect();
        let lo = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let margin = MARGIN as i64;
        let c0 = (lo(&cols).floor() as i64 - margin).max(self.cols.start);
        let c1 = (hi(&cols).ceil() as i64 + margin).min(self.cols.end);
        let r0 = (lo(&rows).floor() as i64 - margin).max(self.rows.start);
        let r1 = (hi(&rows).ceil() as i64 + margin).min(self.rows.end);
        (c0 < c1 && r0 < r1).then_some((c0..c1, r0..r1))
    }

    /// The cells of the lattice window `cols × rows`, NaN where no tile
    /// covers them or a tile holds NoData.
    fn read(&self, cols: &std::ops::Range<i64>, rows: &std::ops::Range<i64>) -> Result<Vec<f64>> {
        let width = (cols.end - cols.start) as usize;
        let height = (rows.end - rows.start) as usize;
        let mut cells = vec![f64::NAN; width * height];
        for tile in &self.tiles {
            let c0 = cols.start.max(tile.col);
            let c1 = cols.end.min(tile.col + tile.cols as i64);
            let r0 = rows.start.max(tile.row);
            let r1 = rows.end.min(tile.row + tile.rows as i64);
            if c0 >= c1 || r0 >= r1 {
                continue;
            }
            let (w, h) = ((c1 - c0) as usize, (r1 - r0) as usize);
            let band = tile
                .dataset
                .rasterband(1)
                .map_err(Error::gdal(&tile.path))?;
            let buffer: Buffer<f64> = band
                .read_as::<f64>(
                    ((c0 - tile.col) as isize, (r0 - tile.row) as isize),
                    (w, h),
                    (w, h),
                    None,
                )
                .map_err(Error::gdal(&tile.path))?;
            let values = buffer.into_shape_and_vec().1;
            for (i, row) in values.chunks_exact(w).enumerate() {
                let at = ((r0 - rows.start) as usize + i) * width + (c0 - cols.start) as usize;
                for (cell, &value) in cells[at..at + w].iter_mut().zip(row) {
                    if Some(value) != tile.nodata && !value.is_nan() {
                        *cell = value;
                    }
                }
            }
        }
        Ok(cells)
    }
}

/// What a warp wrote.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpStats {
    /// Cells holding a value.
    pub valid_cells: u64,
    /// Sum of the values written (Float64).
    pub sum: f64,
}

/// Warp `sources` (tiles of one product) onto `grid` with `resample`, into
/// a new GeoTIFF `target` of cell type `T`: each value through `convert`,
/// `nodata` where the sources hold none.
///
/// The chunks depend on the grid and the sources only (a chunk's source
/// mosaic holds at most about [`MOSAIC_CELLS`] cells), and the kernel scale
/// is fixed for the whole grid: the output is the same whatever the memory
/// budget and thread count.
#[allow(clippy::too_many_arguments)]
pub fn warp_to_grid<T: GdalType + Copy>(
    sources: &[PathBuf],
    grid: &GridSpec,
    resample: Resample,
    target: &Path,
    nodata: f64,
    convert: impl Fn(f64) -> T,
    threads: usize,
    cancelled: Cancelled<'_>,
) -> Result<WarpStats> {
    warp_in_chunks(
        sources, grid, resample, target, nodata, convert, None, threads, cancelled,
    )
}

/// Cells of a chunk's source mosaic, at most about (128 MiB of Float64).
pub const MOSAIC_CELLS: usize = 1 << 24;

#[allow(clippy::too_many_arguments)]
fn warp_in_chunks<T: GdalType + Copy>(
    sources: &[PathBuf],
    grid: &GridSpec,
    resample: Resample,
    target: &Path,
    nodata: f64,
    convert: impl Fn(f64) -> T,
    side: Option<usize>,
    threads: usize,
    cancelled: Cancelled<'_>,
) -> Result<WarpStats> {
    let mosaic = Mosaic::open(sources)?;
    let traditional = |srs: &SpatialRef| {
        let mut srs = srs.clone();
        srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
        srs
    };
    let (grid_srs, source_srs) = (traditional(&grid.srs), traditional(&mosaic.srs));
    let to_source =
        CoordTransform::new(&grid_srs, &source_srs).map_err(Error::gdal(&mosaic.tiles[0].path))?;
    let scale = mosaic.cells_per_cell(grid, &to_source)?;
    // Chunk side such that its source window stays within MOSAIC_CELLS.
    let ratio = scale.0.max(scale.1);
    let side = side.unwrap_or_else(|| {
        let side = ((MOSAIC_CELLS as f64).sqrt() / ratio - 2.0 * MARGIN as f64).floor();
        (side.max(16.0) as usize).min(1024)
    });
    let out = GeoTiff::create::<T>(target, grid, predictor::<T>(), threads, nodata)?;
    let mut stats = WarpStats {
        valid_cells: 0,
        sum: 0.0,
    };
    let mut buffer: Vec<T> = Vec::new();
    for_each_window(
        0..grid.rows,
        0..grid.cols,
        (side, side),
        cancelled,
        |rows, cols| {
            let frame = grid.window(rows.clone(), cols.clone());
            let values = match mosaic.window_under(&frame, &to_source) {
                Some((src_cols, src_rows)) => {
                    let cells = mosaic.read(&src_cols, &src_rows)?;
                    warp_chunk(
                        &mosaic, &src_cols, &src_rows, cells, &frame, resample, scale,
                    )?
                }
                None => vec![f64::NAN; frame.cols * frame.rows],
            };
            buffer.clear();
            for value in values {
                if value.is_finite() {
                    stats.valid_cells += 1;
                    stats.sum += value;
                    buffer.push(convert(value));
                } else {
                    buffer.push(convert(nodata));
                }
            }
            out.write(rows, cols, &mut buffer)
        },
    )?;
    out.close()?;
    Ok(stats)
}

fn predictor<T: GdalType>() -> u8 {
    use gdal::raster::GdalDataType::*;
    match T::datatype() {
        Float32 | Float64 => 3,
        _ => 2,
    }
}

/// One `GDALWarp` from the mosaic window to the chunk `frame`: its values,
/// row-major, NaN where nothing came.
fn warp_chunk(
    mosaic: &Mosaic,
    cols: &std::ops::Range<i64>,
    rows: &std::ops::Range<i64>,
    cells: Vec<f64>,
    frame: &GridSpec,
    resample: Resample,
    (along_x, along_y): (f64, f64),
) -> Result<Vec<f64>> {
    let path = &mosaic.tiles[0].path;
    let mem = DriverManager::get_driver_by_name("MEM").map_err(Error::gdal(path))?;
    let (width, height) = (
        (cols.end - cols.start) as usize,
        (rows.end - rows.start) as usize,
    );
    let mut source = mem
        .create_with_band_type::<f64, _>("", width, height, 1)
        .map_err(Error::gdal(path))?;
    let [x0, ew, _, y0, _, ns] = mosaic.geo_transform;
    source
        .set_geo_transform(&[
            x0 + cols.start as f64 * ew,
            ew,
            0.0,
            y0 + rows.start as f64 * ns,
            0.0,
            ns,
        ])
        .map_err(Error::gdal(path))?;
    source
        .set_spatial_ref(&mosaic.srs)
        .map_err(Error::gdal(path))?;
    {
        let mut band = source.rasterband(1).map_err(Error::gdal(path))?;
        band.set_no_data_value(Some(f64::NAN))
            .map_err(Error::gdal(path))?;
        let mut buffer = Buffer::new((width, height), cells);
        band.write((0, 0), (width, height), &mut buffer)
            .map_err(Error::gdal(path))?;
    }
    let mut target = mem
        .create_with_band_type::<f64, _>("", frame.cols, frame.rows, 1)
        .map_err(Error::gdal(path))?;
    target
        .set_geo_transform(&frame.geo_transform)
        .map_err(Error::gdal(path))?;
    target
        .set_spatial_ref(&frame.srs)
        .map_err(Error::gdal(path))?;
    {
        let mut band = target.rasterband(1).map_err(Error::gdal(path))?;
        band.set_no_data_value(Some(f64::NAN))
            .map_err(Error::gdal(path))?;
        band.fill(f64::NAN, None).map_err(Error::gdal(path))?;
    }
    // GDAL scales its kernel by destination cells per source cell; left to
    // itself it measures that per call, from each chunk's windows.
    let arguments = [
        "-r".to_owned(),
        resample.gdal().to_owned(),
        "-wo".to_owned(),
        "INIT_DEST=NO_DATA".to_owned(),
        "-wo".to_owned(),
        format!("XSCALE={}", 1.0 / along_x),
        "-wo".to_owned(),
        format!("YSCALE={}", 1.0 / along_y),
        "-et".to_owned(),
        "0".to_owned(),
        "-q".to_owned(),
    ];
    let arguments: Vec<CString> = arguments
        .iter()
        .map(|a| CString::new(a.as_str()).map_err(|_| Error::invalid("warp argument with NUL")))
        .collect::<Result<_>>()?;
    let mut argv: Vec<*mut c_char> = arguments
        .iter()
        .map(|a| a.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    // SAFETY: `argv` is a NULL-terminated list of C strings alive for the
    // call; GDAL copies them into the options, freed below.
    let options =
        unsafe { gdal_sys::GDALWarpAppOptionsNew(argv.as_mut_ptr(), std::ptr::null_mut()) };
    if options.is_null() {
        return Err(Error::invalid("invalid warp options"));
    }
    let mut handles = [source.c_dataset()];
    let mut usage_error: c_int = 0;
    // SAFETY: both handles are valid while `source` and `target` live; with
    // a destination dataset GDALWarp returns that same handle, still owned
    // by `target`.
    let result = unsafe {
        gdal_sys::GDALWarp(
            std::ptr::null(),
            target.c_dataset(),
            1,
            handles.as_mut_ptr(),
            options,
            &mut usage_error,
        )
    };
    // SAFETY: `options` came from GDALWarpAppOptionsNew and is freed once.
    unsafe { gdal_sys::GDALWarpAppOptionsFree(options) };
    if result.is_null() || usage_error != 0 {
        return Err(gdal_error(path, "GDALWarp failed"));
    }
    let band = target.rasterband(1).map_err(Error::gdal(path))?;
    let buffer: Buffer<f64> = band
        .read_as::<f64>(
            (0, 0),
            (frame.cols, frame.rows),
            (frame.cols, frame.rows),
            None,
        )
        .map_err(Error::gdal(path))?;
    Ok(buffer.into_shape_and_vec().1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Extent;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    /// A WGS 84 raster `cols × rows` at `res` degrees from `(lon0, lat0)`
    /// (north-west corner), values from `f(col, row)` on the global lattice.
    fn wgs84_tile(
        path: &Path,
        lon0: f64,
        lat0: f64,
        res: f64,
        (cols, rows): (usize, usize),
        f: impl Fn(f64, f64) -> f64,
    ) {
        let driver = DriverManager::get_driver_by_name("GTiff").unwrap();
        let mut ds = driver
            .create_with_band_type::<f64, _>(path, cols, rows, 1)
            .unwrap();
        ds.set_geo_transform(&[lon0, res, 0.0, lat0, 0.0, -res])
            .unwrap();
        ds.set_spatial_ref(&SpatialRef::from_epsg(4326).unwrap())
            .unwrap();
        let mut values = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                let lon = lon0 + (c as f64 + 0.5) * res;
                let lat = lat0 - (r as f64 + 0.5) * res;
                values.push(f(lon, lat));
            }
        }
        let mut band = ds.rasterband(1).unwrap();
        band.set_no_data_value(Some(-9999.0)).unwrap();
        band.write((0, 0), (cols, rows), &mut Buffer::new((cols, rows), values))
            .unwrap();
    }

    fn grid() -> GridSpec {
        // UTM 36N, about 33.09–33.46° E, 9.04–9.41° N, 1 km cells.
        let srs = SpatialRef::from_epsg(32636).unwrap();
        let extent = Extent {
            xmin: 510_000.0,
            ymin: 1_000_000.0,
            xmax: 550_000.0,
            ymax: 1_040_000.0,
        };
        GridSpec::from_extent(extent, 1000.0, srs, (0.0, 0.0), u64::MAX).unwrap()
    }

    fn read(path: &Path) -> Vec<f64> {
        let ds = Dataset::open(path).unwrap();
        let (cols, rows) = ds.raster_size();
        let band = ds.rasterband(1).unwrap();
        band.read_as::<f64>((0, 0), (cols, rows), (cols, rows), None)
            .unwrap()
            .into_shape_and_vec()
            .1
    }

    fn surface(lon: f64, lat: f64) -> f64 {
        100.0 + 50.0 * lon + 30.0 * lat
    }

    /// Two tiles split at 33.25° E give exactly the same raster as one tile,
    /// and small chunks the same values: no seam.
    #[test]
    fn tiles_warp_without_seams() {
        let dir = tmp();
        let res = 1.0 / 120.0;
        let one = dir.path().join("one.tif");
        wgs84_tile(&one, 33.0, 9.5, res, (60, 60), surface);
        let west = dir.path().join("west.tif");
        let east = dir.path().join("east.tif");
        wgs84_tile(&west, 33.0, 9.5, res, (30, 60), surface);
        wgs84_tile(&east, 33.25, 9.5, res, (30, 60), surface);
        let g = grid();
        for resample in [
            Resample::Bilinear,
            Resample::Nearest,
            Resample::Mode,
            Resample::Sum,
        ] {
            let mut outputs = Vec::new();
            for (name, sources, side) in [
                ("single.tif", vec![one.clone()], None),
                ("tiled.tif", vec![west.clone(), east.clone()], None),
                ("small.tif", vec![west.clone(), east.clone()], Some(7)),
            ] {
                let target = dir.path().join(format!("{resample:?}-{name}"));
                warp_in_chunks::<f64>(
                    &sources,
                    &g,
                    resample,
                    &target,
                    -9999.0,
                    |v| v,
                    side,
                    1,
                    &|| false,
                )
                .expect("warp");
                outputs.push(read(&target));
            }
            assert_eq!(outputs[0], outputs[1], "{resample:?}: seam between tiles");
            // Another chunking moves chunk origins: the same values up to the
            // last bits of the coordinates (the default chunking depends on
            // the grid and the sources only, so outputs are reproducible).
            for (a, b) in outputs[0].iter().zip(&outputs[2]) {
                assert!(
                    (a - b).abs() <= 1e-9 * a.abs().max(1.0),
                    "{resample:?}: seam between chunks: {a} vs {b}"
                );
            }
            assert!(outputs[0].iter().any(|v| *v != -9999.0));
        }
    }

    /// Bilinear on a plane in degrees gives the plane at cell centres.
    #[test]
    fn bilinear_reproduces_a_plane() {
        let dir = tmp();
        let src = dir.path().join("plane.tif");
        wgs84_tile(&src, 33.0, 9.5, 1.0 / 120.0, (60, 60), surface);
        let g = grid();
        let target = dir.path().join("out.tif");
        warp_to_grid::<f64>(
            &[src],
            &g,
            Resample::Bilinear,
            &target,
            -9999.0,
            |v| v,
            1,
            &|| false,
        )
        .expect("warp");
        let values = read(&target);
        let to_wgs84 = CoordTransform::new(
            &{
                let mut s = g.srs.clone();
                s.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
                s
            },
            &{
                let mut s = SpatialRef::from_epsg(4326).unwrap();
                s.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
                s
            },
        )
        .unwrap();
        let (r, c) = (g.rows / 2, g.cols / 2);
        let (x, y) = g.cell_centre(r, c);
        let (mut xs, mut ys) = ([x], [y]);
        to_wgs84
            .transform_coords(&mut xs, &mut ys, &mut [])
            .unwrap();
        let expected = surface(xs[0], ys[0]);
        assert!(
            ((values[r * g.cols + c] - expected) / expected).abs() < 1e-5,
            "{} vs {expected}",
            values[r * g.cols + c]
        );
    }

    /// Sum keeps a total: people on a grid fully inside the source.
    #[test]
    fn sum_keeps_the_total() {
        let dir = tmp();
        let res = 1.0 / 1200.0; // about 90 m
        let src = dir.path().join("pop.tif");
        wgs84_tile(&src, 33.0, 9.5, res, (600, 600), |lon, lat| {
            ((lon * 97.0).sin() + 1.0) * ((lat * 53.0).cos() + 1.5)
        });
        let g = grid();
        let target = dir.path().join("sum.tif");
        let stats = warp_to_grid::<f64>(
            std::slice::from_ref(&src),
            &g,
            Resample::Sum,
            &target,
            -9999.0,
            |v| v,
            1,
            &|| false,
        )
        .expect("warp");
        // Source total over the grid's footprint: cells whose centre falls
        // in the grid.
        let ds = Dataset::open(&src).unwrap();
        let values = ds
            .rasterband(1)
            .unwrap()
            .read_as::<f64>((0, 0), (600, 600), (600, 600), None)
            .unwrap()
            .into_shape_and_vec()
            .1;
        let to_grid = CoordTransform::new(
            &{
                let mut s = SpatialRef::from_epsg(4326).unwrap();
                s.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
                s
            },
            &{
                let mut s = g.srs.clone();
                s.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
                s
            },
        )
        .unwrap();
        let (mut xs, mut ys): (Vec<f64>, Vec<f64>) = (0..360_000)
            .map(|i| {
                (
                    33.0 + ((i % 600) as f64 + 0.5) * res,
                    9.5 - ((i / 600) as f64 + 0.5) * res,
                )
            })
            .unzip();
        to_grid.transform_coords(&mut xs, &mut ys, &mut []).unwrap();
        let inside: f64 = (0..360_000)
            .filter(|&i| g.locate(xs[i], ys[i]).is_some())
            .map(|i| values[i])
            .sum();
        let relative = (stats.sum - inside).abs() / inside;
        assert!(
            relative < 5e-3,
            "warped {} vs source {inside}: {relative}",
            stats.sum
        );
    }

    #[test]
    fn mismatched_tiles_and_cancellation_are_errors() {
        let dir = tmp();
        let a = dir.path().join("a.tif");
        let b = dir.path().join("b.tif");
        wgs84_tile(&a, 33.0, 9.5, 1.0 / 120.0, (10, 10), surface);
        wgs84_tile(&b, 33.0001, 9.5, 1.0 / 120.0, (10, 10), surface);
        let g = grid();
        let target = dir.path().join("x.tif");
        assert!(
            warp_to_grid::<f64>(
                &[a.clone(), b],
                &g,
                Resample::Nearest,
                &target,
                -9999.0,
                |v| v,
                1,
                &|| false
            )
            .is_err()
        );
        assert!(
            warp_to_grid::<f64>(
                &[],
                &g,
                Resample::Nearest,
                &target,
                -9999.0,
                |v| v,
                1,
                &|| false
            )
            .is_err()
        );
        let target = dir.path().join("y.tif");
        let error = warp_to_grid::<f64>(
            &[a],
            &g,
            Resample::Nearest,
            &target,
            -9999.0,
            |v| v,
            1,
            &|| true,
        )
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled), "{error:?}");
    }
}
