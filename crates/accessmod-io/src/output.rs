//! Outputs: atomic output directory, streamed GeoTIFFs and CSV, JSON
//! documents, and the travel-time rasters.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use accessmod_core::solver::SolverCell;
use accessmod_core::store::TiledGrid;
use gdal::raster::{Buffer, GdalType, RasterCreationOptions};
use gdal::{Dataset, DriverManager};
use serde::Serialize;

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

/// GeoTIFF block side, cells. Outputs are written one block at a time.
pub const BLOCK: usize = 256;

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
        // On Unix, rename(2) atomically replaces an empty target directory.
        // Windows refuses to rename over a directory: remove the empty target
        // first (not atomic, but it was checked empty just above).
        if let Err(error) = std::fs::rename(&self.path, &self.target) {
            if !(cfg!(windows) && self.target.is_dir()) {
                return Err(Error::file(&self.target)(error));
            }
            std::fs::remove_dir(&self.target).map_err(Error::file(&self.target))?;
            std::fs::rename(&self.path, &self.target).map_err(Error::file(&self.target))?;
        }
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

/// A single-band GeoTIFF on the grid, written block by block: tiled,
/// DEFLATE-compressed, with the grid's geotransform and CRS.
#[derive(Debug)]
pub struct GeoTiff {
    dataset: Dataset,
    path: PathBuf,
}

impl GeoTiff {
    /// Create `path` with cells of type `T`, the given TIFF `predictor`
    /// (2 for integers, 3 for floats) and NoData value.
    pub fn create<T: GdalType>(
        path: &Path,
        grid: &GridSpec,
        predictor: u8,
        threads: usize,
        nodata: f64,
    ) -> Result<Self> {
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
        Ok(Self {
            dataset,
            path: path.to_path_buf(),
        })
    }

    /// Write `data` (`width × height`, row-major) at `(col, row)`; `data`
    /// comes back empty, its allocation kept for the next block.
    pub fn write<T: GdalType + Copy>(
        &self,
        (col, row): (usize, usize),
        (width, height): (usize, usize),
        data: &mut Vec<T>,
    ) -> Result<()> {
        let mut buffer = Buffer::new((width, height), std::mem::take(data));
        let result = self.dataset.rasterband(1).and_then(|mut band| {
            band.write((col as isize, row as isize), (width, height), &mut buffer)
        });
        *data = buffer.into_shape_and_vec().1;
        data.clear();
        result.map_err(Error::gdal(&self.path))
    }

    /// Flush and close the file.
    pub fn close(self) -> Result<()> {
        self.dataset.close().map_err(Error::gdal(&self.path))
    }
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
    let time_tif =
        GeoTiff::create::<f64>(&dir.join(TRAVEL_TIME_FILE), grid, 3, threads, FLOAT_NODATA)?;
    let nearest_tif = GeoTiff::create::<u32>(
        &dir.join(NEAREST_FILE),
        grid,
        2,
        threads,
        f64::from(NEAREST_NODATA),
    )?;
    let distance_tif =
        GeoTiff::create::<f64>(&dir.join(DISTANCE_FILE), grid, 3, threads, FLOAT_NODATA)?;
    let direction_tif = if with_direction {
        Some(GeoTiff::create::<u8>(
            &dir.join(DIRECTION_FILE),
            grid,
            2,
            threads,
            f64::from(DIRECTION_NODATA),
        )?)
    } else {
        None
    };

    // One GeoTIFF block at a time, buffers reused: memory does not grow
    // with the grid width.
    let mut cells = vec![SolverCell::UNREACHED; BLOCK];
    let mut time = Vec::with_capacity(BLOCK * BLOCK);
    let mut nearest = Vec::with_capacity(BLOCK * BLOCK);
    let mut distance = Vec::with_capacity(BLOCK * BLOCK);
    let mut direction = Vec::with_capacity(if with_direction { BLOCK * BLOCK } else { 0 });
    for row0 in (0..rows).step_by(BLOCK) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let height = BLOCK.min(rows - row0);
        for col0 in (0..cols).step_by(BLOCK) {
            let width = BLOCK.min(cols - col0);
            for row in row0..row0 + height {
                state.read_row(row, col0, &mut cells[..width])?;
                for cell in &cells[..width] {
                    let reached = cell.is_reached();
                    time.push(if reached { cell.time_s } else { FLOAT_NODATA });
                    nearest.push(if reached {
                        cell.nearest
                    } else {
                        NEAREST_NODATA
                    });
                    distance.push(if reached {
                        cell.distance_m
                    } else {
                        FLOAT_NODATA
                    });
                    if with_direction {
                        direction.push(if reached { cell.step } else { DIRECTION_NODATA });
                    }
                }
            }
            let at = (col0, row0);
            let size = (width, height);
            time_tif.write(at, size, &mut time)?;
            nearest_tif.write(at, size, &mut nearest)?;
            distance_tif.write(at, size, &mut distance)?;
            if let Some(tif) = &direction_tif {
                tif.write(at, size, &mut direction)?;
            }
        }
    }
    for tif in [time_tif, nearest_tif, distance_tif]
        .into_iter()
        .chain(direction_tif)
    {
        tif.close()?;
    }
    Ok(())
}

/// Write `value` as pretty JSON to `path`, with a trailing newline.
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| Error::File {
        path: path.to_path_buf(),
        source: std::io::Error::other(e),
    })?;
    text.push('\n');
    std::fs::write(path, text).map_err(Error::file(path))
}

/// Last step of every run: write `job.resolved.json` and `run.json` into
/// the staging directory, then move it into place unless cancelled.
pub fn finish(
    staging: Staging,
    job: &impl Serialize,
    report: &impl Serialize,
    cancelled: &dyn Fn() -> bool,
) -> Result<PathBuf> {
    write_json(&staging.path().join("job.resolved.json"), job)?;
    write_json(&staging.path().join("run.json"), report)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }
    staging.commit()
}

/// CSV field: identifiers as text, quoted when needed; floats in shortest
/// round-trip form.
pub fn csv_field(value: &serde_json::Value) -> String {
    let text = match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// Streaming CSV file: a header, then one line per record.
#[derive(Debug)]
pub struct CsvWriter {
    path: PathBuf,
    out: std::io::BufWriter<std::fs::File>,
}

impl CsvWriter {
    /// Create `path` and write the `header` line.
    pub fn create(path: &Path, header: &str) -> Result<Self> {
        let file = std::fs::File::create(path).map_err(Error::file(path))?;
        let mut writer = Self {
            path: path.to_path_buf(),
            out: std::io::BufWriter::new(file),
        };
        writer.line(header)?;
        Ok(writer)
    }

    /// Write one line of already formatted fields (see [`csv_field`]).
    pub fn record(&mut self, fields: &[String]) -> Result<()> {
        self.line(&fields.join(","))
    }

    fn line(&mut self, text: &str) -> Result<()> {
        writeln!(self.out, "{text}").map_err(Error::file(&self.path))
    }

    /// Flush the file.
    pub fn finish(mut self) -> Result<()> {
        self.out.flush().map_err(Error::file(&self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_are_quoted_when_needed() {
        assert_eq!(csv_field(&serde_json::json!("HC 1")), "HC 1");
        assert_eq!(csv_field(&serde_json::json!("a,b")), "\"a,b\"");
        assert_eq!(
            csv_field(&serde_json::json!("say \"hi\"")),
            "\"say \"\"hi\"\"\""
        );
        assert_eq!(csv_field(&serde_json::json!(12)), "12");
        assert_eq!(csv_field(&serde_json::Value::Null), "");
    }
}
