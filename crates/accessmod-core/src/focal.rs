//! Focal operations on disk-backed grids: Euclidean distance to source
//! cells (GRASS `r.grow.distance`, `r.buffer`) and sums over a circular
//! neighbourhood (`r.neighbors -c method=sum`), as AccessMod 5 uses them to
//! build suitability maps and exclusion areas.
//!
//! Both read and write the grid row by row: memory is a few rows of the
//! grid width, whatever its height, and results do not depend on the tile
//! cache.

use crate::store::{StoreError, TiledGrid};

/// Replace every cell of `grid` by the Euclidean distance between its centre
/// and the centre of the nearest source cell, in map units for cells of
/// `dx × dy`. On input, source cells hold `0.0` and other cells
/// `f64::INFINITY`; cells stay infinite when the grid has no source.
///
/// Exact distance transform in two separable passes (Felzenszwalb and
/// Huttenlocher): nearest source along each column with two row sweeps,
/// then the lower envelope of parabolas along each row. `check` runs once
/// per row and per sweep (cancellation).
pub fn distance_to_sources<E: From<StoreError>>(
    grid: &mut TiledGrid<f64>,
    dx: f64,
    dy: f64,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let geometry = grid.geometry();
    let (rows, cols) = (geometry.rows(), geometry.cols());
    let mut row_values = vec![0.0; cols];
    // Rows to the nearest source above, then below (infinite when none).
    let mut last: Vec<Option<usize>> = vec![None; cols];
    for row in 0..rows {
        check()?;
        grid.read_row(row, 0, &mut row_values)?;
        for (col, value) in row_values.iter_mut().enumerate() {
            if *value == 0.0 {
                last[col] = Some(row);
            }
            *value = last[col].map_or(f64::INFINITY, |r| (row - r) as f64);
        }
        grid.write_row(row, 0, &row_values)?;
    }
    last.fill(None);
    for row in (0..rows).rev() {
        check()?;
        grid.read_row(row, 0, &mut row_values)?;
        for (col, value) in row_values.iter_mut().enumerate() {
            if *value == 0.0 {
                last[col] = Some(row);
            }
            let below = last[col].map_or(f64::INFINITY, |r| (r - row) as f64);
            let rows_away = value.min(below);
            // Squared distance along the column, map units.
            *value = if rows_away.is_finite() {
                (dy * rows_away) * (dy * rows_away)
            } else {
                f64::INFINITY
            };
        }
        grid.write_row(row, 0, &row_values)?;
    }

    // Along rows: D(c) = min_j (dx (c − j))² + g(j).
    let mut sites = vec![0usize; cols];
    let mut bounds = vec![0.0f64; cols + 1];
    let mut out = vec![0.0; cols];
    for row in 0..rows {
        check()?;
        grid.read_row(row, 0, &mut row_values)?;
        let g = &row_values;
        let parabola = |j: usize, c: usize| {
            let d = dx * (c as f64 - j as f64);
            d * d + g[j]
        };
        let mut k: isize = -1;
        for q in 0..cols {
            if !g[q].is_finite() {
                continue;
            }
            let fq = g[q] + (dx * q as f64) * (dx * q as f64);
            loop {
                if k < 0 {
                    k = 0;
                    sites[0] = q;
                    bounds[0] = f64::NEG_INFINITY;
                    bounds[1] = f64::INFINITY;
                    break;
                }
                let p = sites[k as usize];
                let fp = g[p] + (dx * p as f64) * (dx * p as f64);
                let s = (fq - fp) / (2.0 * dx * dx * (q as f64 - p as f64));
                if s <= bounds[k as usize] {
                    k -= 1;
                    continue;
                }
                k += 1;
                sites[k as usize] = q;
                bounds[k as usize] = s;
                bounds[k as usize + 1] = f64::INFINITY;
                break;
            }
        }
        if k < 0 {
            out.fill(f64::INFINITY);
        } else {
            let mut j = 0;
            for (c, value) in out.iter_mut().enumerate() {
                while bounds[j + 1] < c as f64 {
                    j += 1;
                }
                *value = parabola(sites[j], c).sqrt();
            }
        }
        grid.write_row(row, 0, &out)?;
    }
    Ok(())
}

/// Sum of the values of `input` within `radius` cells of each cell (cells
/// whose offset `(dr, dc)` has `dr² + dc² <= radius²`), written to `output`:
/// NaN cells are skipped, and the sum is NaN when every cell of the disc is
/// NaN (GRASS `r.neighbors -c method=sum size=2·radius+1`). Holds
/// `2·radius + 1` rows in memory; `check` runs once per row.
pub fn circular_sum<E: From<StoreError>>(
    input: &mut TiledGrid<f64>,
    output: &mut TiledGrid<f64>,
    radius: usize,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let geometry = input.geometry();
    let (rows, cols) = (geometry.rows(), geometry.cols());
    let r = radius as isize;
    // Column half-width of the disc at each row offset.
    let half: Vec<usize> = (-r..=r)
        .map(|dr| {
            let mut w = 0;
            while ((w + 1) * (w + 1)) as isize + dr * dr <= r * r {
                w += 1;
            }
            w
        })
        .collect();
    let mut out = vec![0.0; cols];
    let read = |grid: &mut TiledGrid<f64>, row: isize| -> Result<Option<Vec<f64>>, StoreError> {
        if row < 0 || row as usize >= rows {
            return Ok(None);
        }
        let mut values = vec![0.0; cols];
        grid.read_row(row as usize, 0, &mut values)?;
        Ok(Some(values))
    };
    let mut rows_held: Vec<Option<Vec<f64>>> = Vec::with_capacity(2 * radius + 1);
    for dr in -r..=r {
        rows_held.push(read(input, dr)?);
    }
    for row in 0..rows {
        check()?;
        for (c, value) in out.iter_mut().enumerate() {
            let (mut sum, mut any) = (0.0, false);
            for (k, held) in rows_held.iter().enumerate() {
                let Some(values) = held else { continue };
                let w = half[k];
                let first = c.saturating_sub(w);
                let last = (c + w).min(cols - 1);
                for v in &values[first..=last] {
                    if !v.is_nan() {
                        sum += v;
                        any = true;
                    }
                }
            }
            *value = if any { sum } else { f64::NAN };
        }
        output.write_row(row, 0, &out)?;
        rows_held.remove(0);
        rows_held.push(read(input, row as isize + r + 1)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: usize, cols: usize, values: &[f64]) -> TiledGrid<f64> {
        let geometry = crate::store::TileGeometry::new(rows, cols, 4).expect("geometry");
        let dir = std::env::temp_dir();
        let mut g = TiledGrid::new(geometry, 0.0, 2, &dir, "focal").expect("grid");
        for (i, v) in values.iter().enumerate() {
            g.set(i / cols, i % cols, *v).expect("set");
        }
        g
    }

    fn values(g: &mut TiledGrid<f64>) -> Vec<f64> {
        let geometry = g.geometry();
        let mut all = Vec::new();
        for r in 0..geometry.rows() {
            for c in 0..geometry.cols() {
                all.push(g.get(r, c).expect("get"));
            }
        }
        all
    }

    #[test]
    fn distance_matches_brute_force() {
        let (rows, cols) = (9, 11);
        let sources = [(1usize, 2usize), (7, 9), (4, 5)];
        let mut input = vec![f64::INFINITY; rows * cols];
        for &(r, c) in &sources {
            input[r * cols + c] = 0.0;
        }
        let mut g = grid(rows, cols, &input);
        distance_to_sources::<StoreError>(&mut g, 100.0, 50.0, || Ok(())).expect("edt");
        let got = values(&mut g);
        for r in 0..rows {
            for c in 0..cols {
                let want = sources
                    .iter()
                    .map(|&(sr, sc)| {
                        let (x, y) = (
                            100.0 * (c as f64 - sc as f64),
                            50.0 * (r as f64 - sr as f64),
                        );
                        (x * x + y * y).sqrt()
                    })
                    .fold(f64::INFINITY, f64::min);
                assert!((got[r * cols + c] - want).abs() < 1e-9, "({r}, {c})");
            }
        }
        let mut empty = grid(3, 3, &[f64::INFINITY; 9]);
        distance_to_sources::<StoreError>(&mut empty, 1.0, 1.0, || Ok(())).expect("edt");
        assert!(values(&mut empty).iter().all(|v| v.is_infinite()));
    }

    #[test]
    fn circular_sum_skips_nan() {
        let nan = f64::NAN;
        #[rustfmt::skip]
        let input = [
            1.0, 2.0, nan, 4.0,
            nan, nan, nan, nan,
            5.0, nan, 7.0, 8.0,
        ];
        let mut g = grid(3, 4, &input);
        let mut out = grid(3, 4, &[0.0; 12]);
        circular_sum::<StoreError>(&mut g, &mut out, 1, || Ok(())).expect("sum");
        let got = values(&mut out);
        // Radius 1: the cell and its four edge neighbours.
        assert_eq!(got[0], 3.0);
        assert_eq!(got[5], 2.0); // (1, 1): above 2, others NaN
        assert_eq!(got[6], 7.0); // (1, 2): below 7
        let mut zero = grid(3, 4, &[0.0; 12]);
        circular_sum::<StoreError>(&mut g, &mut zero, 0, || Ok(())).expect("sum");
        let same = values(&mut zero);
        assert!(
            same.iter()
                .zip(&input)
                .all(|(a, b)| a == b || (a.is_nan() && b.is_nan()))
        );
    }
}
