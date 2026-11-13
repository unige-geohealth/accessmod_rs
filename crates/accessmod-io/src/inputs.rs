//! Reading the DEM and land cover into the solver's tiled input grid.

use std::collections::BTreeMap;
use std::path::Path;

use accessmod_core::solver::InputCell;
use accessmod_core::store::TiledGrid;
use gdal::Dataset;
use serde::Serialize;

use crate::error::Result;
use crate::grid::{Alignment, GridSpec};
use crate::preview::ValueCount;
use crate::raster::{AlignedBand, CHUNK_COLS, for_each_chunk, integer_nodata};

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

impl InputSummary {
    /// The class counts of a summary from the cells per land-cover class
    /// (stored at import), without reading the raster: what
    /// [`load_inputs`] counts in `class_cells`, `unmapped_classes` and
    /// `unmapped_other_cells`; the other counts stay zero.
    pub fn from_class_counts(counts: &[ValueCount], slots: &BTreeMap<i64, u16>) -> Self {
        let mut summary = Self::default();
        for &ValueCount { value, count } in counts {
            if slots.contains_key(&value) {
                summary.class_cells.insert(value, count);
            } else if summary.unmapped_classes.len() < MAX_TRACKED_CLASSES {
                summary.unmapped_classes.insert(value, count);
            } else {
                summary.unmapped_other_cells += count;
            }
        }
        summary
    }
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
    let dem_band = AlignedBand::new(dem, dem_path, grid.alignment(), grid)?;
    let dem_nodata = dem_band.nodata();
    let lc_band = AlignedBand::new(land_cover, land_cover_path, alignment, grid)?;
    let lc_nodata = integer_nodata(lc_band.nodata());
    let (rows, cols) = (grid.rows, grid.cols);

    let mut summary = InputSummary {
        cells: (rows * cols) as u64,
        ..InputSummary::default()
    };
    let mut classes = vec![None::<i64>; CHUNK_COLS.min(cols)];
    for_each_chunk(0..rows, 0..cols, cancelled, |strip, chunk| {
        let dem_window = dem_band
            .read::<f64>(strip.clone(), chunk.clone())?
            .expect("the DEM covers its own grid");
        let lc_window = lc_band.read::<i64>(strip.clone(), chunk.clone())?;
        for row in strip {
            let classes = &mut classes[..chunk.len()];
            classes.fill(None);
            if let Some(window) = &lc_window
                && let Some(values) = window.row(row)
            {
                for (k, value) in values.iter().enumerate() {
                    if Some(*value) != lc_nodata {
                        classes[window.first_col - chunk.start + k] = Some(*value);
                    }
                }
            }
            let elevations = dem_window.row(row).expect("the window holds the strip");
            for (k, (class, z)) in classes.iter().zip(elevations).enumerate() {
                let cell = classify(*z, dem_nodata, *class, slots, &mut summary);
                inputs.set(row, chunk.start + k, cell)?;
            }
        }
        Ok(())
    })?;
    Ok(summary)
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
