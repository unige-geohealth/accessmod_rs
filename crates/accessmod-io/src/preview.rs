//! Preview data of a raster, for maps and legends of national grids:
//!
//! - overviews in an external `<raster>.ovr` file, which GDAL reads with
//!   the raster: the raster itself is never modified, so canonical outputs
//!   keep their bytes;
//! - summary statistics (range, mean, histogram, and the distinct values of
//!   a class raster), so a legend never scans a whole grid.
//!
//! Both read the whole raster once or twice; callers run them when a
//! dataset enters a project, not while drawing.

use std::collections::BTreeMap;
use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};

use gdal::Dataset;
use gdal::raster::GdalDataType;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::Cancelled;
use crate::error::{Error, Result};
use crate::raster::{for_each_chunk, valid_value};

/// Bins of [`RasterStats::histogram`].
pub const HISTOGRAM_BINS: usize = 64;

/// Distinct values kept in [`RasterStats::values`]; beyond, a raster is not
/// treated as classes.
pub const MAX_DISTINCT: usize = 1024;

/// Overviews stop once the largest side fits in this many pixels (one map
/// tile).
pub const OVERVIEW_MIN_SIDE: usize = 256;

/// How overview pixels summarise the cells below them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Resampling {
    /// One of the cells: classes and identifiers (land cover, cost
    /// allocation).
    Nearest,
    /// Mean of the valid cells: continuous values (elevation, travel time,
    /// population).
    Average,
}

impl Resampling {
    fn gdal_name(self) -> &'static str {
        match self {
            Resampling::Nearest => "NEAREST",
            Resampling::Average => "AVERAGE",
        }
    }
}

/// Count of one distinct value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValueCount {
    /// Cell value.
    pub value: i64,
    /// Cells holding it.
    pub count: u64,
}

/// Summary of band 1 of a raster. NoData and NaN cells are not valid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RasterStats {
    /// Cells of the raster.
    pub cells: u64,
    /// Cells with a value.
    pub valid_cells: u64,
    /// Smallest value; none without valid cells.
    pub min: Option<f64>,
    /// Largest value; none without valid cells.
    pub max: Option<f64>,
    /// Mean value; none without valid cells.
    pub mean: Option<f64>,
    /// Valid cells in [`HISTOGRAM_BINS`] equal bins over `[min, max]` (the
    /// last bin includes `max`); empty without valid cells.
    pub histogram: Vec<u64>,
    /// Distinct values in ascending order, for an integer raster with at
    /// most [`MAX_DISTINCT`] of them (classes); none otherwise.
    pub values: Option<Vec<ValueCount>>,
}

/// Overviews and statistics of a raster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Preview {
    /// Overview factors written to the `.ovr` file (2, 4, …); empty for a
    /// raster small enough to draw directly.
    pub overviews: Vec<usize>,
    /// Statistics of band 1.
    pub stats: RasterStats,
}

/// Build the overviews of `path` and compute its statistics.
pub fn build(
    path: &Path,
    resampling: Resampling,
    threads: usize,
    cancelled: Cancelled<'_>,
) -> Result<Preview> {
    let stats = statistics(path, cancelled)?;
    let overviews = build_overviews(path, resampling, threads, cancelled)?;
    Ok(Preview { overviews, stats })
}

/// Path of the external overviews of `path`.
pub fn overview_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".ovr");
    PathBuf::from(name)
}

/// Overview factors for a raster of `cols × rows`: 2, 4, … until the
/// largest side fits in [`OVERVIEW_MIN_SIDE`].
pub fn overview_factors(cols: usize, rows: usize) -> Vec<usize> {
    let side = cols.max(rows);
    let mut factors = Vec::new();
    let mut factor = 2;
    while side.div_ceil(factor / 2) > OVERVIEW_MIN_SIDE {
        factors.push(factor);
        factor *= 2;
    }
    factors
}

/// Write the overviews of `path` to [`overview_path`], replacing any; the
/// factors written. Nothing is left behind on failure or cancellation.
pub fn build_overviews(
    path: &Path,
    resampling: Resampling,
    threads: usize,
    cancelled: Cancelled<'_>,
) -> Result<Vec<usize>> {
    let ovr = overview_path(path);
    remove_if_exists(&ovr)?;
    let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
    let (cols, rows) = dataset.raster_size();
    let factors = overview_factors(cols, rows);
    if factors.is_empty() {
        return Ok(factors);
    }
    let result = with_overview_options(threads, || {
        write_overviews(&dataset, path, resampling, &factors, cancelled)
    });
    drop(dataset);
    if result.is_err() {
        remove_if_exists(&ovr)?;
    }
    result.map(|()| factors)
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::file(path)(e)),
    }
}

/// Run `f` with the overview creation options set for this thread only.
fn with_overview_options<T>(threads: usize, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let options = [
        ("COMPRESS_OVERVIEW", "DEFLATE".to_owned()),
        ("BIGTIFF_OVERVIEW", "IF_SAFER".to_owned()),
        ("GDAL_NUM_THREADS", threads.max(1).to_string()),
    ];
    for (key, value) in &options {
        gdal::config::set_thread_local_config_option(key, value).map_err(Error::gdal(*key))?;
    }
    let result = f();
    for (key, _) in &options {
        gdal::config::clear_thread_local_config_option(key).map_err(Error::gdal(*key))?;
    }
    result
}

/// GDAL progress callback: stop when the flag behind `arg` is raised.
extern "C" fn keep_going(_complete: f64, _message: *const c_char, arg: *mut c_void) -> c_int {
    // SAFETY: `arg` is the `&Cancelled` passed by `write_overviews`, alive
    // for the whole GDALBuildOverviews call.
    let cancelled = unsafe { &*(arg as *const Cancelled<'_>) };
    c_int::from(!cancelled())
}

fn write_overviews(
    dataset: &Dataset,
    path: &Path,
    resampling: Resampling,
    factors: &[usize],
    cancelled: Cancelled<'_>,
) -> Result<()> {
    let mut levels: Vec<c_int> = factors
        .iter()
        .map(|&f| c_int::try_from(f).map_err(|_| Error::invalid("overview factor overflows")))
        .collect::<Result<_>>()?;
    let mut bands: Vec<c_int> = vec![1];
    let method = CString::new(resampling.gdal_name()).expect("method names have no NUL byte");
    let mut flag: Cancelled<'_> = cancelled;
    // SAFETY: the dataset handle is valid while `dataset` lives; `levels`,
    // `bands`, `method` and `flag` outlive the call; `keep_going` only reads
    // `flag`.
    let status = unsafe {
        gdal_sys::GDALBuildOverviews(
            dataset.c_dataset(),
            method.as_ptr(),
            levels.len() as c_int,
            levels.as_mut_ptr(),
            bands.len() as c_int,
            bands.as_mut_ptr(),
            Some(keep_going),
            (&mut flag as *mut Cancelled<'_>).cast::<c_void>(),
        )
    };
    if cancelled() {
        return Err(Error::Cancelled);
    }
    if status != gdal_sys::CPLErr::CE_None {
        return Err(Error::Gdal {
            path: path.to_path_buf(),
            source: gdal::errors::GdalError::CplError {
                class: status,
                number: 0,
                msg: "GDALBuildOverviews failed".into(),
            },
        });
    }
    Ok(())
}

/// Statistics of band 1 of `path`, read in bounded chunks (two passes).
pub fn statistics(path: &Path, cancelled: Cancelled<'_>) -> Result<RasterStats> {
    let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
    let band = dataset.rasterband(1).map_err(Error::gdal(path))?;
    let (cols, rows) = dataset.raster_size();
    let nodata = band.no_data_value();
    let integer = !matches!(
        band.band_type(),
        GdalDataType::Float32 | GdalDataType::Float64 | GdalDataType::Unknown
    );

    let read = |rows_range: std::ops::Range<usize>,
                cols_range: std::ops::Range<usize>|
     -> Result<Vec<f64>> {
        let size = (cols_range.len(), rows_range.len());
        band.read_as::<f64>(
            (cols_range.start as isize, rows_range.start as isize),
            size,
            size,
            None,
        )
        .map(|buffer| buffer.into_shape_and_vec().1)
        .map_err(Error::gdal(path))
    };

    // Pass 1: range, sum, distinct values.
    let mut valid = 0u64;
    let (mut min, mut max, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0f64);
    let mut distinct: Option<BTreeMap<i64, u64>> = integer.then(BTreeMap::new);
    for_each_chunk(0..rows, 0..cols, cancelled, |r, c| {
        for value in read(r, c)? {
            let Some(v) = valid_value(value, nodata) else {
                continue;
            };
            valid += 1;
            min = min.min(v);
            max = max.max(v);
            sum += v;
            if let Some(map) = &mut distinct {
                *map.entry(v as i64).or_insert(0) += 1;
                if map.len() > MAX_DISTINCT {
                    distinct = None;
                }
            }
        }
        Ok(())
    })?;
    if valid == 0 {
        return Ok(RasterStats {
            cells: (cols * rows) as u64,
            valid_cells: 0,
            min: None,
            max: None,
            mean: None,
            histogram: Vec::new(),
            values: None,
        });
    }

    // Pass 2: histogram over [min, max].
    let mut histogram = vec![0u64; HISTOGRAM_BINS];
    let width = max - min;
    for_each_chunk(0..rows, 0..cols, cancelled, |r, c| {
        for value in read(r, c)? {
            if let Some(v) = valid_value(value, nodata) {
                histogram[histogram_bin(v, min, width)] += 1;
            }
        }
        Ok(())
    })?;

    Ok(RasterStats {
        cells: (cols * rows) as u64,
        valid_cells: valid,
        min: Some(min),
        max: Some(max),
        mean: Some(sum / valid as f64),
        histogram,
        values: distinct.map(|map| {
            map.into_iter()
                .map(|(value, count)| ValueCount { value, count })
                .collect()
        }),
    })
}

/// Bin of `value` in [`HISTOGRAM_BINS`] equal bins from `min` over `width`.
fn histogram_bin(value: f64, min: f64, width: f64) -> usize {
    if width <= 0.0 {
        return 0;
    }
    let bin = ((value - min) / width * HISTOGRAM_BINS as f64) as usize;
    bin.min(HISTOGRAM_BINS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factors_stop_once_a_tile_holds_the_raster() {
        assert!(overview_factors(256, 100).is_empty());
        assert_eq!(overview_factors(300, 200), vec![2]);
        assert_eq!(overview_factors(1000, 1000), vec![2, 4]);
        // 20 000 cells: 20 000 / 128 = 157 fits a tile.
        assert_eq!(
            overview_factors(20_000, 20_000),
            vec![2, 4, 8, 16, 32, 64, 128]
        );
    }

    #[test]
    fn bins_cover_the_range_and_include_the_maximum() {
        assert_eq!(histogram_bin(0.0, 0.0, 10.0), 0);
        assert_eq!(histogram_bin(10.0, 0.0, 10.0), HISTOGRAM_BINS - 1);
        assert_eq!(histogram_bin(5.0, 0.0, 10.0), HISTOGRAM_BINS / 2);
        assert_eq!(histogram_bin(3.0, 3.0, 0.0), 0);
    }
}
