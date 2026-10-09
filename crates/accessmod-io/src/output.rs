//! Canonical outputs: atomic output directory and streamed GeoTIFFs.

use std::path::{Path, PathBuf};

use accessmod_core::solver::SolverCell;
use accessmod_core::store::TiledGrid;
use gdal::raster::{Buffer, GdalType, RasterCreationOptions};
use gdal::{Dataset, DriverManager};

use crate::error::{Error, Result};
use crate::grid::GridSpec;

/// NoData of `travel_time_seconds.tif` and `path_distance_m.tif`.
pub const FLOAT_NODATA: f64 = -9999.0;
/// NoData of `nearest_facility.tif` (ordinals start at 1).
pub const NEAREST_NODATA: u32 = 0;
/// NoData of `direction.tif`; 0 marks a facility cell.
pub const DIRECTION_NODATA: u8 = 255;

/// Travel time output file name.
pub const TRAVEL_TIME_FILE: &str = "travel_time_seconds.tif";
/// Cost allocation output file name.
pub const NEAREST_FILE: &str = "nearest_facility.tif";
/// Path distance output file name.
pub const DISTANCE_FILE: &str = "path_distance_m.tif";
/// Direction output file name.
pub const DIRECTION_FILE: &str = "direction.tif";

/// GeoTIFF block side, cells.
const BLOCK: usize = 256;

/// Output directory being written. Files go to a hidden sibling directory
/// that replaces the target on [`Staging::commit`]; dropping an uncommitted
/// staging directory removes it (failure or cancellation).
#[derive(Debug)]
pub struct Staging {
    path: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl Staging {
    /// Check that `target` does not exist or is an empty directory, and
    /// create the staging directory next to it.
    pub fn create(target: &Path) -> Result<Self> {
        check_output_dir(target)?;
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if !parent.is_dir() {
            return Err(Error::invalid(format!(
                "output_dir {}: parent directory does not exist",
                target.display()
            )));
        }
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "output".to_owned());
        let path = parent.join(format!(".{name}.accessmod-tmp-{}", std::process::id()));
        std::fs::create_dir(&path).map_err(Error::file(&path))?;
        Ok(Self {
            path,
            target: target.to_path_buf(),
            committed: false,
        })
    }

    /// Staging directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Move the staging directory to the target.
    pub fn commit(mut self) -> Result<PathBuf> {
        check_output_dir(&self.target)?;
        if self.target.exists() {
            std::fs::remove_dir(&self.target).map_err(Error::file(&self.target))?;
        }
        std::fs::rename(&self.path, &self.target).map_err(Error::file(&self.target))?;
        self.committed = true;
        Ok(self.target.clone())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.committed {
            // Best effort: nothing else can be done during unwinding.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// `target` must not exist or be an empty directory: no implicit overwrite.
pub fn check_output_dir(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(target).map_err(|_| {
        Error::invalid(format!(
            "output_dir {} exists and is not a directory",
            target.display()
        ))
    })?;
    if entries.next().is_some() {
        return Err(Error::invalid(format!(
            "output_dir {} is not empty (outputs are never overwritten)",
            target.display()
        )));
    }
    Ok(())
}

fn create_geotiff<T: GdalType>(
    path: &Path,
    grid: &GridSpec,
    predictor: u8,
    threads: usize,
    nodata: f64,
) -> Result<Dataset> {
    let driver = DriverManager::get_driver_by_name("GTiff").map_err(Error::gdal(path))?;
    let mut options = RasterCreationOptions::new();
    for (key, value) in [
        ("TILED", "YES".to_owned()),
        ("BLOCKXSIZE", BLOCK.to_string()),
        ("BLOCKYSIZE", BLOCK.to_string()),
        ("COMPRESS", "DEFLATE".to_owned()),
        ("PREDICTOR", predictor.to_string()),
        ("BIGTIFF", "IF_SAFER".to_owned()),
        ("NUM_THREADS", threads.max(1).to_string()),
    ] {
        options
            .set_name_value(key, &value)
            .map_err(Error::gdal(path))?;
    }
    let mut dataset = driver
        .create_with_band_type_with_options::<T, _>(path, grid.cols, grid.rows, 1, &options)
        .map_err(Error::gdal(path))?;
    dataset
        .set_geo_transform(&grid.geo_transform)
        .map_err(Error::gdal(path))?;
    dataset
        .set_spatial_ref(&grid.srs)
        .map_err(Error::gdal(path))?;
    dataset
        .rasterband(1)
        .and_then(|mut band| band.set_no_data_value(Some(nodata)))
        .map_err(Error::gdal(path))?;
    Ok(dataset)
}

fn write_strip<T: GdalType + Copy>(
    dataset: &Dataset,
    path: &Path,
    start: usize,
    cols: usize,
    data: &[T],
) -> Result<()> {
    let height = data.len() / cols;
    let mut buffer = Buffer::new((cols, height), data.to_vec());
    dataset
        .rasterband(1)
        .and_then(|mut band| band.write((0, start as isize), (cols, height), &mut buffer))
        .map_err(Error::gdal(path))
}

/// Write the canonical rasters from the solver state into `dir`.
pub fn write_rasters(
    dir: &Path,
    grid: &GridSpec,
    state: &mut TiledGrid<SolverCell>,
    with_direction: bool,
    threads: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let (rows, cols) = (grid.rows, grid.cols);
    let time_path = dir.join(TRAVEL_TIME_FILE);
    let nearest_path = dir.join(NEAREST_FILE);
    let distance_path = dir.join(DISTANCE_FILE);
    let direction_path = dir.join(DIRECTION_FILE);
    let time_ds = create_geotiff::<f64>(&time_path, grid, 3, threads, FLOAT_NODATA)?;
    let nearest_ds =
        create_geotiff::<u32>(&nearest_path, grid, 2, threads, f64::from(NEAREST_NODATA))?;
    let distance_ds = create_geotiff::<f64>(&distance_path, grid, 3, threads, FLOAT_NODATA)?;
    let direction_ds = if with_direction {
        Some(create_geotiff::<u8>(
            &direction_path,
            grid,
            2,
            threads,
            f64::from(DIRECTION_NODATA),
        )?)
    } else {
        None
    };

    let mut row_cells = vec![SolverCell::UNREACHED; cols];
    for start in (0..rows).step_by(BLOCK) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let height = BLOCK.min(rows - start);
        let mut time = Vec::with_capacity(height * cols);
        let mut nearest = Vec::with_capacity(height * cols);
        let mut distance = Vec::with_capacity(height * cols);
        let mut direction = Vec::with_capacity(if with_direction { height * cols } else { 0 });
        for row in start..start + height {
            state.read_row(row, 0, &mut row_cells)?;
            for cell in &row_cells {
                if cell.is_reached() {
                    time.push(cell.time_s);
                    nearest.push(cell.nearest);
                    distance.push(cell.distance_m);
                    if with_direction {
                        direction.push(cell.step);
                    }
                } else {
                    time.push(FLOAT_NODATA);
                    nearest.push(NEAREST_NODATA);
                    distance.push(FLOAT_NODATA);
                    if with_direction {
                        direction.push(DIRECTION_NODATA);
                    }
                }
            }
        }
        write_strip(&time_ds, &time_path, start, cols, &time)?;
        write_strip(&nearest_ds, &nearest_path, start, cols, &nearest)?;
        write_strip(&distance_ds, &distance_path, start, cols, &distance)?;
        if let Some(ds) = &direction_ds {
            write_strip(ds, &direction_path, start, cols, &direction)?;
        }
    }
    for (dataset, path) in [
        (time_ds, time_path),
        (nearest_ds, nearest_path),
        (distance_ds, distance_path),
    ]
    .into_iter()
    .chain(direction_ds.map(|ds| (ds, direction_path)))
    {
        dataset.close().map_err(Error::gdal(path))?;
    }
    Ok(())
}

/// Write `value` as pretty JSON to `path`, with a trailing newline.
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| Error::File {
        path: path.to_path_buf(),
        source: std::io::Error::other(e),
    })?;
    text.push('\n');
    std::fs::write(path, text).map_err(Error::file(path))
}
