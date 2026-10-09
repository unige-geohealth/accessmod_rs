//! Parity of the committed bicycle LUT with the AccessMod 5 oracle.
//!
//! The oracle table is the `float` array compiled into the pinned
//! `r.walk.accessmod` binary, extracted by `scripts/oracle/export.sh` into
//! `_oracle/bicycle-lut/`. The test skips, with a message, when the export is
//! absent.

use std::path::PathBuf;

use accessmod_core::mobility::bicycle::{self, SLOPE_COUNT, SPEED_COUNT, TABLE_LEN};

fn oracle_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_oracle/bicycle-lut/r.walk.accessmod")
}

/// Locate the LUT in the binary: row 0 (flat speed 0) is 441 zeros, row 1
/// starts with values close to ours. Returns the decoded oracle table.
fn find_table(binary: &[u8], ours: &[f32]) -> Option<Vec<f32>> {
    let row_bytes = SLOPE_COUNT * 4;
    let table_bytes = TABLE_LEN * 4;
    let close = |offset: usize, expected: f32| {
        let v = f32::from_le_bytes(binary[offset..offset + 4].try_into().ok()?);
        Some((v - expected).abs() <= 1e-4 * expected.abs().max(1.0))
    };
    (0..binary.len().checked_sub(table_bytes)?)
        .step_by(4)
        .find(|&start| {
            binary[start..start + row_bytes].iter().all(|&b| b == 0)
                && (0..8)
                    .all(|k| close(start + row_bytes + 4 * k, ours[SLOPE_COUNT + k]) == Some(true))
        })
        .and_then(|start| bicycle::decode_table(&binary[start..start + table_bytes]))
}

#[test]
fn committed_table_matches_oracle() {
    let path = oracle_binary();
    let Ok(binary) = std::fs::read(&path) else {
        eprintln!(
            "SKIPPED: oracle binary not found at {}; run scripts/oracle/export.sh",
            path.display()
        );
        return;
    };
    let ours = bicycle::table();
    let oracle = find_table(&binary, ours).expect("bicycle LUT found in the oracle binary");

    let mut differing = Vec::new();
    for (index, (a, b)) in ours.iter().zip(&oracle).enumerate() {
        if a.to_bits() != b.to_bits() {
            differing.push((index / SLOPE_COUNT, index % SLOPE_COUNT, *a, *b));
        }
    }
    let max_ulps = differing
        .iter()
        .map(|&(_, _, a, b)| (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs())
        .max()
        .unwrap_or(0);
    eprintln!(
        "{} of {} cells differ from the oracle (max {} ulp)",
        differing.len(),
        SPEED_COUNT * SLOPE_COUNT,
        max_ulps
    );
    for (i, j, a, b) in differing.iter().take(10) {
        eprintln!("  row {i} col {j}: ours {a:.9} oracle {b:.9}");
    }
    // Bit-identical as of 5.9.2-beta.6: the model root is solved to full f64
    // precision here and to 1e-11 m/s by R `uniroot`, but both values round
    // to the same `%.8f` literal and f32. A difference means a model change.
    assert!(
        differing.is_empty(),
        "LUT differs from the oracle (max {max_ulps} ulp)"
    );
}
