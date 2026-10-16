//! Shared steps of the capacity analyses (geographic coverage, scaling up):
//! the residual population, one search per facility grouped in travel-time
//! bands with its reached cells spooled to disk, the catchment polygon and
//! the removal of the population covered (AccessMod 5
//! `amAnalysisCatchment.R`; rules in `accessmod_core::capacity`).

use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use accessmod_core::capacity::{Band, Catchment};
use accessmod_core::cost::CellMobility;
use accessmod_core::solver::{Facility, Flow, InputCell, SolverCell, SolverConfig, solve_until};
use accessmod_core::store::{TileGeometry, TiledGrid};
use gdal::vector::Geometry;

use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::output::{BLOCK, FLOAT_NODATA, GeoTiff, ScratchFile};
use crate::raster::{AlignedBand, for_each_block, for_each_window};
use crate::vector::polygonize;

/// Reached cells of one search, spooled to a scratch file so that memory
/// does not grow with the catchment: cell index and band.
pub(crate) struct CellSpool {
    file: ScratchFile,
    writer: Option<BufWriter<std::fs::File>>,
    pub(crate) count: u64,
}

impl CellSpool {
    pub(crate) fn new(dir: &Path) -> Result<Self> {
        let mut spool = Self {
            file: ScratchFile::new(dir, "coverage", "cells"),
            writer: None,
            count: 0,
        };
        spool.clear()?;
        Ok(spool)
    }

    pub(crate) fn path(&self) -> &Path {
        self.file.path()
    }

    pub(crate) fn clear(&mut self) -> Result<()> {
        let file = std::fs::File::create(self.path()).map_err(Error::file(self.path()))?;
        self.writer = Some(BufWriter::new(file));
        self.count = 0;
        Ok(())
    }

    pub(crate) fn push(&mut self, cell: u64, band: u32) -> std::io::Result<()> {
        let writer = self.writer.as_mut().expect("spool open for writing");
        writer.write_all(&cell.to_le_bytes())?;
        writer.write_all(&band.to_le_bytes())?;
        self.count += 1;
        Ok(())
    }

    /// Visit the spooled cells in the order they were pushed; cancellation
    /// is checked every 65 536 cells.
    pub(crate) fn for_each(
        &mut self,
        cancelled: &dyn Fn() -> bool,
        mut visit: impl FnMut(u64, u32) -> Result<()>,
    ) -> Result<()> {
        if let Some(mut writer) = self.writer.take() {
            writer.flush().map_err(Error::file(self.path()))?;
        }
        let file = std::fs::File::open(self.path()).map_err(Error::file(self.path()))?;
        let mut reader = BufReader::new(file);
        let mut entry = [0u8; 12];
        for k in 0..self.count {
            if k % 65_536 == 0 && cancelled() {
                return Err(Error::Cancelled);
            }
            reader
                .read_exact(&mut entry)
                .map_err(Error::file(self.path()))?;
            let cell = u64::from_le_bytes(entry[..8].try_into().expect("8 bytes"));
            let band = u32::from_le_bytes(entry[8..].try_into().expect("4 bytes"));
            visit(cell, band)?;
        }
        Ok(())
    }
}

/// Residual population per band of one search from `facility`, the
/// reached cells spooled. Cells without population are reached but hold no
/// residual: bands where no cell holds a value are left out (AccessMod 5).
#[allow(clippy::too_many_arguments)] // reason: one private helper, explicit inputs
pub(crate) fn search_bands(
    inputs: &mut TiledGrid<InputCell>,
    scenario: &[CellMobility],
    facility: Facility,
    config: &SolverConfig,
    band_s: f64,
    residual: &mut TiledGrid<f64>,
    spool: &mut CellSpool,
    state_tiles: usize,
    tmp_dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Band>> {
    let geometry = inputs.geometry();
    let cols = geometry.cols();
    let mut state = TiledGrid::new(
        geometry,
        SolverCell::UNREACHED,
        state_tiles.max(1),
        tmp_dir,
        "coverage",
    )?;
    spool.clear()?;
    let mut sums: BTreeMap<u32, f64> = BTreeMap::new();
    let mut failure: Option<Error> = None;
    solve_until(
        inputs,
        scenario,
        &[facility],
        config,
        &mut state,
        cancelled,
        &mut |r, c, cell| {
            let band = (cell.time_s / band_s).ceil() as u32;
            let result = spool
                .push((r * cols + c) as u64, band)
                .map_err(Error::file(spool.path()))
                .and_then(|()| residual.get(r, c).map_err(Error::from));
            match result {
                Ok(value) if !value.is_nan() => *sums.entry(band).or_default() += value,
                Ok(_) => {}
                Err(error) => {
                    failure = Some(error);
                    return Flow::Stop;
                }
            }
            Flow::Continue
        },
    )?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(sums
        .into_iter()
        .map(|(band, population)| Band { band, population })
        .collect())
}

/// Polygon of the spooled cells within band `limit`: their bounding box is
/// burnt into a disk-backed mask, written to a scratch GeoTIFF and
/// polygonised. Memory stays bounded by the tile cache and one polygon.
pub(crate) fn catchment_polygon(
    spool: &mut CellSpool,
    limit: u32,
    grid: &GridSpec,
    tmp_dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Geometry>> {
    let cols = grid.cols as u64;
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    spool.for_each(cancelled, |cell, band| {
        if band <= limit {
            let (r, c) = ((cell / cols) as usize, (cell % cols) as usize);
            bbox = Some(bbox.map_or((r, r, c, c), |(r0, r1, c0, c1)| {
                (r0.min(r), r1.max(r), c0.min(c), c1.max(c))
            }));
        }
        Ok(())
    })?;
    let Some((r0, r1, c0, c1)) = bbox else {
        return Ok(None);
    };
    let window = grid.window(r0..r1 + 1, c0..c1 + 1);
    let geometry = TileGeometry::new(
        window.rows,
        window.cols,
        accessmod_core::store::DEFAULT_TILE,
    )?;
    // A small cache: the mask is written once in spool order, then read
    // block by block.
    let mut mask = TiledGrid::new(geometry, 0u8, 64, tmp_dir, "catchment")?;
    spool.for_each(cancelled, |cell, band| {
        if band <= limit {
            let (r, c) = ((cell / cols) as usize, (cell % cols) as usize);
            mask.set(r - r0, c - c0, 1)?;
        }
        Ok(())
    })?;
    let scratch = ScratchFile::new(tmp_dir, "catchment", "tif");
    let path = scratch.path().to_path_buf();
    let tif = GeoTiff::create::<u8>(&path, &window, 2, 1, 0.0)?;
    let mut cells = vec![0u8; BLOCK];
    let mut block = Vec::with_capacity(BLOCK * BLOCK);
    for_each_window(
        0..window.rows,
        0..window.cols,
        (BLOCK, BLOCK),
        cancelled,
        |r, c| {
            for row in r.clone() {
                mask.read_row(row, c.start, &mut cells[..c.len()])?;
                block.extend_from_slice(&cells[..c.len()]);
            }
            tif.write(r, c, &mut block)
        },
    )?;
    tif.close()?;
    let dataset = gdal::Dataset::open(&path).map_err(Error::gdal(&path))?;
    let polygon = polygonize(&dataset, &path);
    drop(dataset);
    polygon
}

/// Population totals of [`init_residual`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct PopulationTotals {
    /// Every value of the raster.
    pub(crate) total: f64,
    /// Values on cells that cannot be entered.
    pub(crate) on_barriers: f64,
    /// Residual population at start (people off barriers).
    pub(crate) off_barriers: f64,
}

/// Fill `residual` from `population` (AccessMod 5 amInitPopResidual,
/// `pop > 0 &&& speed > 0 ? pop : 0` with the null-aware `&&&`): no value
/// without population, 0 for people <= 0 (barriers included), the people
/// off barriers, no value for people on barriers. With `barriers`, also
/// write the population on barriers (amMapPopOnBarrier: every value where
/// the cell cannot be entered), NoData elsewhere.
pub(crate) fn init_residual(
    grid: &GridSpec,
    population: &AlignedBand<'_>,
    inputs: &mut TiledGrid<InputCell>,
    residual: &mut TiledGrid<f64>,
    barriers: Option<&GeoTiff>,
    cancelled: &dyn Fn() -> bool,
) -> Result<PopulationTotals> {
    let mut totals = PopulationTotals::default();
    let mut barriers_block = Vec::with_capacity(BLOCK * BLOCK);
    for_each_block(
        grid,
        (BLOCK, BLOCK),
        &[population],
        None,
        cancelled,
        |block| {
            let width = block.cols.len();
            for i in 0..block.len() {
                let (row, col) = (block.rows.start + i / width, block.cols.start + i % width);
                let Some(people) = block.value(0, i) else {
                    barriers_block.push(FLOAT_NODATA);
                    continue;
                };
                totals.total += people;
                let passable = inputs.get(row, col)?.is_passable();
                if passable {
                    barriers_block.push(FLOAT_NODATA);
                } else {
                    totals.on_barriers += people;
                    barriers_block.push(people);
                }
                let value = if people <= 0.0 {
                    0.0
                } else if passable {
                    people
                } else {
                    continue;
                };
                totals.off_barriers += value;
                residual.set(row, col, value)?;
            }
            match barriers {
                Some(tif) => tif.write(block.rows.clone(), block.cols.clone(), &mut barriers_block),
                None => {
                    barriers_block.clear();
                    Ok(())
                }
            }
        },
    )?;
    Ok(totals)
}

/// Remove from `residual` the population `catchment` covers among the
/// spooled cells of its search: every band up to the inner ring, the
/// fraction of the outer band. Returns the population removed.
pub(crate) fn remove_covered(
    spool: &mut CellSpool,
    residual: &mut TiledGrid<f64>,
    catchment: &Catchment,
    cancelled: &dyn Fn() -> bool,
) -> Result<f64> {
    let cols = residual.geometry().cols() as u64;
    let (inner, outer) = (catchment.inner, catchment.outer);
    let mut removed = 0.0;
    spool.for_each(cancelled, |cell, band| {
        let (row, col) = ((cell / cols) as usize, (cell % cols) as usize);
        let value = residual.get(row, col)?;
        let new = if inner.is_some_and(|i| band <= i) {
            // AccessMod 5 sets 0 even where there is no population.
            0.0
        } else if let Some((_, fraction)) = outer.filter(|(b, _)| *b == band) {
            value - value * fraction
        } else {
            return Ok(());
        };
        if !value.is_nan() {
            removed += value - new;
        }
        residual.set(row, col, new)?;
        Ok(())
    })?;
    Ok(removed)
}

/// Capacity table columns shared by the capacity analyses, from the limit
/// to the catchment case (see [`catchment_fields`]).
pub(crate) const CATCHMENT_COLUMNS: &str = "travel_time_limit_s,population_within_limit,\
     population_band_correlation,catchment_travel_time_s,population_catchment,\
     capacity_realised,capacity_residual,population_catchment_not_covered,\
     population_residual_before,population_residual_after,population_covered_percent,\
     catchment_case";

/// A number in a capacity table; empty when missing.
pub(crate) fn table_number(v: Option<f64>) -> String {
    v.map(|x| format!("{x}")).unwrap_or_default()
}

/// Residual population around one facility and the share covered so far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResidualStep {
    pub(crate) before: f64,
    pub(crate) after: f64,
    pub(crate) covered_percent: f64,
}

/// Values of [`CATCHMENT_COLUMNS`] for one facility.
pub(crate) fn catchment_fields(
    c: &Catchment,
    limit_s: f64,
    band_s: f64,
    step: ResidualStep,
) -> [String; 12] {
    [
        table_number(Some(limit_s)),
        table_number(c.population_within_limit),
        table_number(c.correlation),
        table_number(c.band_limit.map(|b| f64::from(b) * band_s)),
        table_number(c.population_catchment),
        table_number(Some(c.capacity_realised)),
        table_number(Some(c.capacity_residual)),
        table_number(c.population_not_covered()),
        table_number(Some(step.before)),
        table_number(Some(step.after)),
        table_number(Some(step.covered_percent)),
        c.case.map(|k| format!("{k:?}")).unwrap_or_default(),
    ]
}
