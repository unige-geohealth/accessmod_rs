//! Compare two single-band rasters of the same size cell by cell.
//!
//! `cargo run --release -p accessmod-io --example compare_rasters -- <a.tif> <b.tif>`
//!
//! Prints the maximum relative difference over cells valid in both, and the
//! number of cells valid in only one (NoData and NaN are invalid).

use gdal::Dataset;

fn read(path: &str) -> Result<Vec<Option<f64>>, Box<dyn std::error::Error>> {
    let ds = Dataset::open(path)?;
    let band = ds.rasterband(1)?;
    let (cols, rows) = ds.raster_size();
    let nodata = band.no_data_value();
    let buffer = band.read_as::<f64>((0, 0), (cols, rows), (cols, rows), None)?;
    Ok(buffer
        .data()
        .iter()
        .map(|&v| (!v.is_nan() && Some(v) != nodata).then_some(v))
        .collect())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b] = args.as_slice() else {
        return Err("usage: compare_rasters <a> <b>".into());
    };
    let (a, b) = (read(a)?, read(b)?);
    if a.len() != b.len() {
        return Err(format!("sizes differ: {} vs {} cells", a.len(), b.len()).into());
    }
    let mut max_rel = 0.0f64;
    let mut mask_diff = 0usize;
    for (x, y) in a.iter().zip(&b) {
        match (x, y) {
            (Some(x), Some(y)) => {
                let rel = (x - y).abs() / x.abs().max(y.abs()).max(f64::MIN_POSITIVE);
                if x != y {
                    max_rel = max_rel.max(rel);
                }
            }
            (None, None) => {}
            _ => mask_diff += 1,
        }
    }
    println!("{max_rel:.1e} (mask diff {mask_diff})");
    Ok(())
}
