//! Compare two single-band rasters on the same grid cell by cell.
//!
//! `cargo run --release -p accessmod-io --example compare_rasters -- <a.tif> <b.tif>`
//!
//! Prints the maximum relative difference over cells valid in both, and the
//! number of cells valid in only one (NoData and NaN are invalid). Rasters
//! are read in bounded windows, whatever their size.

use std::path::Path;

use accessmod_io::grid::{GridSpec, RasterFrame};
use accessmod_io::raster::{AlignedBand, for_each_chunk, valid_value};
use gdal::Dataset;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b] = args.as_slice() else {
        return Err("usage: compare_rasters <a> <b>".into());
    };
    let (a, b) = (Path::new(a), Path::new(b));
    let (dataset_a, grid) = GridSpec::open_dem(a)?;
    let dataset_b = Dataset::open(b)?;
    if RasterFrame::of(&dataset_b)? != grid.frame() {
        return Err(format!(
            "{} and {} are not on the same grid",
            a.display(),
            b.display()
        )
        .into());
    }
    let band_a = AlignedBand::new(&dataset_a, a, grid.alignment(), &grid)?;
    let band_b = AlignedBand::new(&dataset_b, b, grid.alignment(), &grid)?;
    let (nodata_a, nodata_b) = (band_a.nodata(), band_b.nodata());

    let mut max_rel = 0.0f64;
    let mut mask_diff = 0usize;
    for_each_chunk(0..grid.rows, 0..grid.cols, &|| false, |rows, cols| {
        let window_a = band_a.read::<f64>(rows.clone(), cols.clone())?;
        let window_b = band_b.read::<f64>(rows.clone(), cols)?;
        let (Some(window_a), Some(window_b)) = (window_a, window_b) else {
            return Ok(());
        };
        for row in rows {
            let (row_a, row_b) = (window_a.row(row), window_b.row(row));
            for (&x, &y) in row_a.into_iter().flatten().zip(row_b.into_iter().flatten()) {
                match (valid_value(x, nodata_a), valid_value(y, nodata_b)) {
                    (Some(x), Some(y)) if x != y => {
                        let rel = (x - y).abs() / x.abs().max(y.abs()).max(f64::MIN_POSITIVE);
                        max_rel = max_rel.max(rel);
                    }
                    (Some(_), None) | (None, Some(_)) => mask_diff += 1,
                    _ => {}
                }
            }
        }
        Ok(())
    })?;
    println!("{max_rel:.1e} (mask diff {mask_diff})");
    Ok(())
}
