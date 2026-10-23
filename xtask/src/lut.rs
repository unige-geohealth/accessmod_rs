//! `cargo xtask lut [--check] [--csv <path>]`: regenerate the bicycle LUT of
//! `accessmod5-v1` from the model equations, write it to
//! `crates/accessmod-core/data/` (or, with `--check`, fail if the committed
//! table differs) and check its interpolation error. `--csv` writes an
//! inspection CSV with every branch of the model at each node.

use std::fmt::Write as _;
use std::path::PathBuf;

use accessmod_core::mobility::bicycle::{self, model};

use crate::workspace_root;

const LUT_PATH: &str = "crates/accessmod-core/data/bicycle-lut-accessmod5-v1.f32le";

/// Run the task with its arguments.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut check = false;
    let mut csv: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--csv" => {
                let path = iter.next().ok_or("--csv needs a path")?;
                csv = Some(PathBuf::from(path));
            }
            other => return Err(format!("unknown argument to lut: {other}")),
        }
    }

    let table = bicycle::generate_table();
    let error = bicycle::interpolation_error(&table);
    eprintln!(
        "bicycle LUT: {} x {} cells; interpolation error over {} samples (1-40 km/h): \
         max {:.4} km/h (limit {}), p99 {:.4} km/h (limit {})",
        bicycle::SPEED_COUNT,
        bicycle::SLOPE_COUNT,
        error.samples,
        error.max_kmh,
        bicycle::MAX_ERROR_KMH,
        error.p99_kmh,
        bicycle::P99_ERROR_KMH,
    );
    if !error.is_acceptable() {
        return Err("bicycle LUT interpolation error exceeds its acceptance limits".into());
    }

    let bytes = bicycle::encode_table(&table);
    let path = workspace_root().join(LUT_PATH);
    if check {
        let committed =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if committed != bytes {
            let differing = bicycle::decode_table(&committed)
                .map(|old| {
                    old.iter()
                        .zip(&table)
                        .filter(|(a, b)| a.to_bits() != b.to_bits())
                        .count()
                })
                .map_or("size differs".to_owned(), |n| format!("{n} cells differ"));
            return Err(format!(
                "{} is out of date ({differing}); run `cargo xtask lut`",
                path.display()
            ));
        }
        eprintln!("{} is up to date", path.display());
    } else {
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }

    if let Some(csv) = csv {
        std::fs::write(&csv, inspection_csv(&table))
            .map_err(|e| format!("cannot write {}: {e}", csv.display()))?;
        eprintln!("wrote {}", csv.display());
    }
    Ok(())
}

/// One row per LUT node: every branch of the exact model and the stored
/// `f32` value. Infinite envelope (flat ground, uphill) is left empty.
fn inspection_csv(table: &[f32]) -> String {
    let mut out = String::from(
        "flat_speed_kmh,slope,physics_kmh,safety_envelope_kmh,ride_kmh,hike_kmh,selected_kmh,table_kmh\n",
    );
    for i in 0..bicycle::SPEED_COUNT {
        let flat = bicycle::grid_speed_kmh(i);
        for j in 0..bicycle::SLOPE_COUNT {
            let slope = bicycle::grid_slope(j);
            let c = model::components(flat, slope);
            let envelope = if c.safety_envelope.is_finite() {
                format!("{:.12}", c.safety_envelope)
            } else {
                String::new()
            };
            let _ = writeln!(
                out,
                "{flat:.1},{slope:.3},{:.12},{envelope},{:.12},{:.12},{:.12},{:.8}",
                c.physics,
                c.ride,
                c.hike,
                c.selected,
                table[i * bicycle::SLOPE_COUNT + j],
            );
        }
    }
    out
}
