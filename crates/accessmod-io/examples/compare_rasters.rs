//! Compare two single-band rasters of the same size cell by cell.
//!
//! `cargo run --release -p accessmod-io --example compare_rasters -- <a.tif> <b.tif>`
//!
//! Prints the maximum relative difference over cells valid in both, and the
//! number of cells valid in only one (NoData and NaN are invalid).

use std::path::Path;

use accessmod_io::fixtures::read_valid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b] = args.as_slice() else {
        return Err("usage: compare_rasters <a> <b>".into());
    };
    let (a, b) = (read_valid(Path::new(a)), read_valid(Path::new(b)));
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
