//! Raster primitives on the grid lattice: rasterising lines and polygons,
//! and 3 × 3 neighbourhoods. The building blocks GRASS gives AccessMod 5
//! (`v.to.rast`, `r.mapcalc` neighbour offsets), reused by every analysis.
//!
//! Coordinates are **pixel coordinates of the whole grid**: `x` is the
//! column, `y` the row, cell `(row, col)` covers `[col, col + 1) ×
//! [row, row + 1)`. Callers convert map coordinates once with the grid
//! geotransform; the rasterisers never see a strip or tile offset, so a
//! geometry burns the same cells whatever the memory budget.
//!
//! Rasterisers report column spans `visit(row, first_col, last_col)`
//! (inclusive), clipped to the grid; spans may repeat or overlap, so the
//! caller's update must be idempotent.

use std::ops::ControlFlow;

/// Index range `[first, last]`, clipped to `0 .. n`, of the half-open unit
/// intervals `[k, k + 1)` holding a point of `[lo, hi]`, or of `[lo, hi)`
/// when `open_hi` (the end point itself belongs to the next unit).
fn unit_range(lo: f64, hi: f64, open_hi: bool, n: usize) -> Option<(usize, usize)> {
    let first = lo.floor();
    let mut last = hi.floor();
    if open_hi && hi > lo && hi == last {
        last -= 1.0;
    }
    if last < 0.0 || first >= n as f64 {
        return None;
    }
    Some((first.max(0.0) as usize, last.min(n as f64 - 1.0) as usize))
}

/// Visit the cells a segment touches, row by row, as column spans
/// `visit(row, first_col, last_col)` (inclusive).
///
/// Cells are half-open (`[col, col + 1) × [row, row + 1)`); a cell is
/// touched when it holds a point of the closed segment. Each row's span is
/// computed from the segment's own end points, never from a clipped one, so
/// it does not depend on how the grid is split. Segments with a non-finite
/// end point touch nothing (callers reject them first).
fn segment_spans(
    a: (f64, f64),
    b: (f64, f64),
    rows: usize,
    cols: usize,
    visit: &mut impl FnMut(usize, usize, usize),
) {
    // Same orientation whichever way the line runs, so that interpolation
    // rounds the same way.
    let ((x0, y0), (x1, y1)) = if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    };
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return;
    }
    let (y_lo, y_hi) = (y0.min(y1), y0.max(y1));
    let Some((r_first, r_last)) = unit_range(y_lo, y_hi, false, rows) else {
        return;
    };
    // x on the segment at height y, exact at both end points.
    let x_at = |y: f64| {
        if y == y0 {
            x0
        } else if y == y1 {
            x1
        } else {
            x0 + (y - y0) * (x1 - x0) / (y1 - y0)
        }
    };
    for row in r_first..=r_last {
        let span = if y0 == y1 {
            unit_range(x0.min(x1), x0.max(x1), false, cols)
        } else {
            // Points with y in [top, bottom], or [top, bottom) when the
            // segment goes on into the next row.
            let top = (row as f64).max(y_lo);
            let next_row = (row + 1) as f64;
            let bottom = next_row.min(y_hi);
            let (xa, xb) = (x_at(top), x_at(bottom));
            if bottom == next_row && xb > xa {
                // The open end is the high one: a whole-number x there
                // belongs to the next row only.
                unit_range(xa, xb, true, cols)
            } else {
                unit_range(xa.min(xb), xa.max(xb), false, cols)
            }
        };
        if let Some((c0, c1)) = span {
            visit(row, c0, c1);
        }
    }
}

/// Visit the cells a polyline touches (every segment, see
/// `segment_spans`). Spans of adjacent segments may overlap; painting is
/// idempotent.
pub fn line_spans(
    points: &[(f64, f64)],
    rows: usize,
    cols: usize,
    visit: &mut impl FnMut(usize, usize, usize),
) {
    match points {
        [] => {}
        [single] => segment_spans(*single, *single, rows, cols, visit),
        _ => {
            for pair in points.windows(2) {
                segment_spans(pair[0], pair[1], rows, cols, visit);
            }
        }
    }
}

/// Visit a thin (8-connected) trace of a polyline, as one-cell spans
/// `visit(row, col, col)`, as `v.to.rast` without `-d` ([`line_spans`] gives
/// every touched cell, as `-d`). Each segment steps along its major axis
/// (x on ties) through every cell between those of its end points, and
/// takes the cell of the line (extended) at that cell's centre line;
/// checked cell for cell against GRASS on the AccessMod 5 demo roads.
/// Cells outside the grid are skipped; a trace never depends on how the
/// grid is split.
pub fn thin_line_cells(
    points: &[(f64, f64)],
    rows: usize,
    cols: usize,
    visit: &mut impl FnMut(usize, usize, usize),
) {
    let cell = |(x, y): (f64, f64)| (x.floor() as i64, y.floor() as i64);
    let mut emit = |c: i64, r: i64| {
        if r >= 0 && c >= 0 && (r as usize) < rows && (c as usize) < cols {
            visit(r as usize, c as usize, c as usize);
        }
    };
    if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return;
    }
    match points {
        [] => {}
        [single] => {
            let (c, r) = cell(*single);
            emit(c, r);
        }
        _ => {
            for pair in points.windows(2) {
                let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
                if x0 == x1 && y0 == y1 {
                    let (c, r) = cell(pair[0]);
                    emit(c, r);
                } else if (x1 - x0).abs() >= (y1 - y0).abs() {
                    // Only columns of the grid can emit a cell.
                    let mut k = x0.min(x1).floor().max(0.0);
                    while k <= x0.max(x1).floor().min(cols as f64 - 1.0) {
                        let x = k + 0.5;
                        let y = y0 + (x - x0) * (y1 - y0) / (x1 - x0);
                        emit(k as i64, y.floor() as i64);
                        k += 1.0;
                    }
                } else {
                    let mut k = y0.min(y1).floor().max(0.0);
                    while k <= y0.max(y1).floor().min(rows as f64 - 1.0) {
                        let y = k + 0.5;
                        let x = x0 + (y - y0) * (x1 - x0) / (y1 - y0);
                        emit(x.floor() as i64, k as i64);
                        k += 1.0;
                    }
                }
            }
        }
    }
}

/// Visit the cells whose centre is inside a polygon, as column spans
/// `visit(row, first_col, last_col)` (inclusive).
///
/// `rings` are the exterior ring and its holes, closed or not; the
/// even-odd rule decides inside. A centre exactly on an edge is inside when
/// the edge is its west boundary (half-open on both axes, deterministic).
/// A polygon with a non-finite vertex covers nothing (callers reject it
/// first).
pub fn polygon_spans(
    rings: &[Vec<(f64, f64)>],
    rows: usize,
    cols: usize,
    visit: &mut impl FnMut(usize, usize, usize),
) {
    if !rings
        .iter()
        .flatten()
        .all(|(x, y)| x.is_finite() && y.is_finite())
    {
        return;
    }
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    for &(_, y) in rings.iter().flatten() {
        y_min = y_min.min(y);
        y_max = y_max.max(y);
    }
    if !(y_min.is_finite() && y_max.is_finite()) {
        return;
    }
    // Rows whose centre y + 0.5 lies in [y_min, y_max].
    let first = (y_min - 0.5).ceil().max(0.0);
    let last = (y_max - 0.5).floor().min(rows as f64 - 1.0);
    if last < first {
        return;
    }
    let mut crossings = Vec::new();
    for row in first as usize..=last as usize {
        let yc = row as f64 + 0.5;
        crossings.clear();
        for ring in rings {
            let n = ring.len();
            for i in 0..n {
                let (ax, ay) = ring[i];
                let (bx, by) = ring[(i + 1) % n];
                // Half-open in y: an edge counts when yc is in [min, max).
                if (ay <= yc) != (by <= yc) {
                    crossings.push(ax + (yc - ay) * (bx - ax) / (by - ay));
                }
            }
        }
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.chunks_exact(2) {
            // Centres c + 0.5 in [x_in, x_out).
            let c0 = (pair[0] - 0.5).ceil().max(0.0);
            let c1 = ((pair[1] - 0.5).ceil() - 1.0).min(cols as f64 - 1.0);
            if c1 >= c0 {
                visit(row, c0 as usize, c1 as usize);
            }
        }
    }
}

/// Visit the exact fraction of each cell covered by a polygon, row by row,
/// as `visit(row, first_col, fractions)` (`fractions[k]` is column
/// `first_col + k`), as `exactextractr` does for its weighted sums.
///
/// `rings` are the exterior ring and its holes, closed or not, of either
/// orientation. The area of the polygon inside cell `(r, c)` follows from
/// Green's theorem, `∮ g(x) [r ≤ y < r + 1] dy` with `g` the part of `[c,
/// c + 1]` left of `x`, integrated in closed form on each edge, so the
/// result is exact up to rounding (fractions are clamped to `[0, 1]`). Rows
/// and columns outside the grid are skipped; only rows and columns of the
/// polygon's bounding box are visited; `visit` stops the walk with
/// `ControlFlow::Break`. A polygon with a non-finite vertex covers nothing
/// (callers reject it first).
pub fn polygon_coverage(
    rings: &[Vec<(f64, f64)>],
    rows: usize,
    cols: usize,
    visit: &mut impl FnMut(usize, usize, &[f64]) -> ControlFlow<()>,
) {
    if !rings
        .iter()
        .flatten()
        .all(|(x, y)| x.is_finite() && y.is_finite())
    {
        return;
    }
    // Non-horizontal edges with the orientation weight of their ring:
    // exterior counted positive, holes negative, whatever the winding.
    struct Edge {
        a: (f64, f64),
        b: (f64, f64),
        weight: f64,
        y_min: f64,
        y_max: f64,
    }
    let mut edges = Vec::new();
    let (mut x_min, mut x_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (k, ring) in rings.iter().enumerate() {
        let n = ring.len();
        // Shoelace sum relative to the first vertex: no cancellation
        // between large products.
        let Some(&(x0, y0)) = ring.first() else {
            continue;
        };
        let doubled: f64 = (0..n)
            .map(|i| {
                let (ax, ay) = (ring[i].0 - x0, ring[i].1 - y0);
                let (bx, by) = (ring[(i + 1) % n].0 - x0, ring[(i + 1) % n].1 - y0);
                ax * by - bx * ay
            })
            .sum();
        if doubled == 0.0 {
            continue;
        }
        let weight = if k == 0 { 1.0 } else { -1.0 } * doubled.signum();
        for i in 0..n {
            let a = ring[i];
            let b = ring[(i + 1) % n];
            x_min = x_min.min(a.0);
            x_max = x_max.max(a.0);
            if a.1 != b.1 {
                edges.push(Edge {
                    a,
                    b,
                    weight,
                    y_min: a.1.min(b.1),
                    y_max: a.1.max(b.1),
                });
            }
        }
    }
    let y_min = edges.iter().map(|e| e.y_min).fold(f64::INFINITY, f64::min);
    let y_max = edges
        .iter()
        .map(|e| e.y_max)
        .fold(f64::NEG_INFINITY, f64::max);
    if edges.is_empty() {
        return;
    }
    let first_row = y_min.floor().max(0.0);
    let end_row = y_max.ceil().min(rows as f64);
    let first_col = x_min.floor().max(0.0);
    let end_col = (x_max.floor() + 1.0).min(cols as f64);
    if end_row <= first_row || end_col <= first_col {
        return;
    }
    let (first_col, width) = (first_col as usize, (end_col - first_col) as usize);
    edges.sort_by(|p, q| p.y_min.total_cmp(&q.y_min));
    // Mean over x in [lo, hi] of the part of [c, c + 1] left of x: the
    // pieces below c (0), across [c, c + 1] (linear) and beyond (1), each
    // weighted by its length; stable however short the interval.
    let mean_covered = |lo: f64, hi: f64, c: f64| {
        if hi <= lo {
            return lo.clamp(c, c + 1.0) - c;
        }
        let (a, b) = (lo.max(c), hi.min(c + 1.0));
        let across = if b > a {
            (b - a) / (hi - lo) * ((a + b) / 2.0 - c)
        } else {
            0.0
        };
        let beyond = (hi - lo.max(c + 1.0)).max(0.0) / (hi - lo);
        across + beyond
    };
    let mut fractions = vec![0.0; width];
    // left[k]: area added to every column of index < k.
    let mut left = vec![0.0; width + 1];
    let mut active: Vec<usize> = Vec::new();
    let mut next = 0;
    for row in first_row as usize..end_row as usize {
        let (top, bottom) = (row as f64, row as f64 + 1.0);
        while next < edges.len() && edges[next].y_min < bottom {
            active.push(next);
            next += 1;
        }
        active.retain(|&e| edges[e].y_max > top);
        fractions.fill(0.0);
        left.fill(0.0);
        for &e in &active {
            let Edge { a, b, weight, .. } = edges[e];
            let x_at = |y: f64| a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1);
            let (ya, yb) = (a.1.clamp(top, bottom), b.1.clamp(top, bottom));
            let dy = (yb - ya) * weight;
            if dy == 0.0 {
                continue;
            }
            let (xa, xb) = (x_at(ya), x_at(yb));
            let (lo, hi) = (xa.min(xb), xa.max(xb));
            let full = (lo.floor() - first_col as f64).clamp(0.0, width as f64) as usize;
            left[full] += dy;
            let from = lo.floor().max(first_col as f64) as usize;
            let to = (hi.floor() as i64).min((first_col + width) as i64 - 1);
            for c in from as i64..=to {
                fractions[c as usize - first_col] += dy * mean_covered(lo, hi, c as f64);
            }
        }
        let mut carry = 0.0;
        for k in (0..width).rev() {
            carry += left[k + 1];
            fractions[k] = (fractions[k] + carry).clamp(0.0, 1.0);
        }
        if visit(row, first_col, &fractions).is_break() {
            return;
        }
    }
}

/// Call `visit(col, window)` for every cell of a row with its 3 × 3
/// neighbourhood: `window[1][1]` is the cell, `window[0]` the row above,
/// `window[2]` the row below (row offsets −1, 0, +1 as `r.mapcalc`
/// `map[r, c]`). `above` and `below` are `None` outside the grid; cells
/// outside the grid read as `outside`.
pub fn for_each_window3<T: Copy>(
    above: Option<&[T]>,
    here: &[T],
    below: Option<&[T]>,
    outside: T,
    mut visit: impl FnMut(usize, [[T; 3]; 3]),
) {
    let cols = here.len();
    let at = |row: Option<&[T]>, c: isize| -> T {
        match row {
            Some(values) if c >= 0 && (c as usize) < cols => values[c as usize],
            _ => outside,
        }
    };
    for col in 0..cols {
        let c = col as isize;
        let mut window = [[outside; 3]; 3];
        for (k, dc) in (-1..=1).enumerate() {
            window[0][k] = at(above, c + dc);
            window[1][k] = at(Some(here), c + dc);
            window[2][k] = at(below, c + dc);
        }
        visit(col, window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells_of_line(points: &[(f64, f64)], rows: usize, cols: usize) -> Vec<(usize, usize)> {
        let mut cells = Vec::new();
        line_spans(points, rows, cols, &mut |r, c0, c1| {
            cells.extend((c0..=c1).map(|c| (r, c)));
        });
        cells.sort_unstable();
        cells.dedup();
        cells
    }

    fn cells_of_polygon(
        rings: &[Vec<(f64, f64)>],
        rows: usize,
        cols: usize,
    ) -> Vec<(usize, usize)> {
        let mut cells = Vec::new();
        polygon_spans(rings, rows, cols, &mut |r, c0, c1| {
            cells.extend((c0..=c1).map(|c| (r, c)));
        });
        cells.sort_unstable();
        cells
    }

    #[test]
    fn thin_lines_step_along_the_major_axis() {
        let mut cells = Vec::new();
        thin_line_cells(
            &[(0.5, 0.5), (3.5, 2.5), (3.5, 2.5)],
            4,
            5,
            &mut |r, c, _| cells.push((r, c)),
        );
        assert_eq!(cells, vec![(0, 0), (1, 1), (1, 2), (2, 3), (2, 3)]);
    }

    #[test]
    fn horizontal_and_vertical_lines() {
        assert_eq!(
            cells_of_line(&[(0.5, 1.5), (3.5, 1.5)], 4, 5),
            vec![(1, 0), (1, 1), (1, 2), (1, 3)]
        );
        assert_eq!(
            cells_of_line(&[(2.2, 0.1), (2.2, 2.9)], 4, 5),
            vec![(0, 2), (1, 2), (2, 2)]
        );
    }

    #[test]
    fn diagonal_line_touches_every_crossed_cell() {
        // From (0.5, 0.2) to (2.5, 1.2): slope 1/2, crosses row 1 at x = 2.1.
        assert_eq!(
            cells_of_line(&[(0.5, 0.2), (2.5, 1.2)], 3, 3),
            vec![(0, 0), (0, 1), (0, 2), (1, 2)]
        );
        // Through a corner exactly: diagonal neighbours only.
        assert_eq!(
            cells_of_line(&[(0.5, 0.5), (1.5, 1.5)], 2, 2),
            vec![(0, 0), (1, 1)]
        );
    }

    #[test]
    fn end_points_on_cell_edges_burn_the_cell_they_belong_to() {
        // (1.0, 0.5) belongs to cell (0, 1); (0.5, 2.0) to cell (2, 0).
        assert_eq!(
            cells_of_line(&[(0.5, 0.5), (1.0, 0.5)], 3, 3),
            vec![(0, 0), (0, 1)]
        );
        assert_eq!(
            cells_of_line(&[(0.5, 0.5), (0.5, 2.0)], 3, 3),
            vec![(0, 0), (1, 0), (2, 0)]
        );
        // Leaving a row through a whole-number x on the high side: the
        // crossing point belongs to the next row.
        assert_eq!(
            cells_of_line(&[(0.5, 0.5), (1.5, 1.5)], 2, 2),
            cells_of_line(&[(1.5, 1.5), (0.5, 0.5)], 2, 2)
        );
        // Moving south-west through the corner (1.0, 1.0), which belongs to
        // cell (1, 1).
        assert_eq!(
            cells_of_line(&[(1.5, 0.5), (0.5, 1.5)], 2, 2),
            vec![(0, 1), (1, 0), (1, 1)]
        );
    }

    #[test]
    fn non_finite_coordinates_burn_nothing() {
        assert!(cells_of_line(&[(0.5, 0.5), (f64::NAN, 1.5)], 3, 3).is_empty());
        let ring = vec![(0.0, 0.0), (3.0, 0.0), (f64::NAN, 3.0), (0.0, 3.0)];
        assert!(cells_of_polygon(&[ring], 3, 3).is_empty());
    }

    #[test]
    fn lines_are_clipped_and_points_burn_one_cell() {
        assert_eq!(
            cells_of_line(&[(-3.0, 0.5), (1.5, 0.5)], 2, 2),
            vec![(0, 0), (0, 1)]
        );
        assert!(cells_of_line(&[(5.0, 5.0), (6.0, 7.0)], 2, 2).is_empty());
        assert_eq!(cells_of_line(&[(1.2, 0.7)], 2, 2), vec![(0, 1)]);
    }

    #[test]
    fn line_cells_do_not_depend_on_direction() {
        let a = [(0.3, 0.9), (4.7, 3.2), (1.1, 5.6)];
        let b: Vec<_> = a.iter().rev().copied().collect();
        assert_eq!(cells_of_line(&a, 6, 6), cells_of_line(&b, 6, 6));
    }

    #[test]
    fn polygon_burns_cells_with_centre_inside() {
        // Square (0.4, 0.4) – (2.6, 2.6): centres 0.5, 1.5, 2.5 all inside.
        let square = vec![(0.4, 0.4), (2.6, 0.4), (2.6, 2.6), (0.4, 2.6)];
        assert_eq!(cells_of_polygon(&[square], 4, 4).len(), 9);
        // (0.6, 0.6) – (2.4, 2.4): only centre 1.5 is inside on each axis.
        let small = vec![(0.6, 0.6), (2.4, 0.6), (2.4, 2.4), (0.6, 2.4), (0.6, 0.6)];
        assert_eq!(cells_of_polygon(&[small], 4, 4), vec![(1, 1)]);
    }

    #[test]
    fn polygon_holes_are_left_out() {
        let outer = vec![(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (0.0, 3.0)];
        let hole = vec![(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)];
        let cells = cells_of_polygon(&[outer, hole], 3, 3);
        assert_eq!(cells.len(), 8);
        assert!(!cells.contains(&(1, 1)));
    }

    #[test]
    fn polygon_edges_on_centres_are_half_open() {
        // West edge on centre x = 0.5 is inside, east edge on 2.5 is not;
        // same for north (inside) and south (outside).
        let square = vec![(0.5, 0.5), (2.5, 0.5), (2.5, 2.5), (0.5, 2.5)];
        assert_eq!(
            cells_of_polygon(&[square], 4, 4),
            vec![(0, 0), (0, 1), (1, 0), (1, 1)]
        );
    }

    #[test]
    fn windows_read_outside_cells_as_given() {
        let above = [1, 2, 3];
        let here = [4, 5, 6];
        let mut windows = Vec::new();
        for_each_window3(Some(&above), &here, None, 0, |col, w| {
            windows.push((col, w))
        });
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0], (0, [[0, 1, 2], [0, 4, 5], [0, 0, 0]]));
        assert_eq!(windows[2], (2, [[2, 3, 0], [5, 6, 0], [0, 0, 0]]));
    }

    fn coverage(rings: &[Vec<(f64, f64)>], rows: usize, cols: usize) -> Vec<f64> {
        let mut out = vec![0.0; rows * cols];
        polygon_coverage(rings, rows, cols, &mut |r, c0, fractions| {
            out[r * cols + c0..r * cols + c0 + fractions.len()].copy_from_slice(fractions);
            ControlFlow::Continue(())
        });
        out
    }

    fn assert_close(got: &[f64], want: &[f64]) {
        assert_eq!(got.len(), want.len());
        assert!(
            got.iter().zip(want).all(|(x, y)| (x - y).abs() < 1e-12),
            "{got:?} != {want:?}"
        );
    }

    #[test]
    fn coverage_of_aligned_and_offset_squares() {
        let square = vec![(1.0, 1.0), (3.0, 1.0), (3.0, 2.0), (1.0, 2.0)];
        let mut want = vec![0.0; 12];
        want[5] = 1.0;
        want[6] = 1.0;
        assert_close(&coverage(std::slice::from_ref(&square), 3, 4), &want);
        let reversed: Vec<_> = square.iter().rev().copied().collect();
        assert_close(&coverage(&[reversed], 3, 4), &want);
        // Shifted by half a cell: quarters at the corners, halves between.
        let offset = vec![(0.5, 0.5), (2.5, 0.5), (2.5, 1.5), (0.5, 1.5)];
        assert_close(
            &coverage(&[offset], 2, 3),
            &[0.25, 0.5, 0.25, 0.25, 0.5, 0.25],
        );
        // Partly outside the grid, west and north.
        let outside = vec![(-0.5, -2.0), (1.5, -2.0), (1.5, 1.0), (-0.5, 1.0)];
        assert_close(&coverage(&[outside], 1, 3), &[1.0, 0.5, 0.0]);
    }

    #[test]
    fn coverage_of_triangle_hole_and_slanted_polygon() {
        let triangle = vec![(0.0, 0.0), (2.0, 0.0), (0.0, 2.0)];
        assert_close(&coverage(&[triangle], 2, 2), &[1.0, 0.5, 0.5, 0.0]);
        // A hole with the same winding as its exterior.
        let outer = vec![(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (0.0, 3.0)];
        let hole = vec![(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)];
        let mut want = vec![1.0; 9];
        want[4] = 0.0;
        assert_close(&coverage(&[outer, hole], 3, 3), &want);
        // Fractions of a slanted quadrilateral sum to its area.
        let quad = vec![(1.3, 0.2), (7.7, 1.6), (6.9, 8.4), (0.4, 5.1)];
        let area = (0..4)
            .map(|i| {
                let (ax, ay) = quad[i];
                let (bx, by) = quad[(i + 1) % 4];
                ax * by - bx * ay
            })
            .sum::<f64>()
            / 2.0;
        let fractions = coverage(&[quad], 10, 10);
        assert!(fractions.iter().all(|f| (0.0..=1.0).contains(f)));
        assert!((fractions.iter().sum::<f64>() - area).abs() < 1e-12);
    }

    #[test]
    fn coverage_whatever_the_windings() {
        let outer = vec![(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (0.0, 3.0)];
        let hole = vec![(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)];
        let flip = |r: &Vec<(f64, f64)>| r.iter().rev().copied().collect::<Vec<_>>();
        let mut want = vec![1.0; 9];
        want[4] = 0.0;
        for rings in [
            [outer.clone(), flip(&hole)],
            [flip(&outer), hole.clone()],
            [flip(&outer), flip(&hole)],
        ] {
            assert_close(&coverage(&rings, 3, 3), &want);
        }
    }

    #[test]
    fn coverage_of_a_near_vertical_edge_across_a_column() {
        // A quadrilateral whose east edge leans by 2ε across x = 1 (exact
        // binary values): the cell
        // east of it holds a triangle of area ε / 4 per unit of height.
        let e = 2f64.powi(-33);
        let quad = vec![(0.0, 0.0), (1.0 - e, 0.0), (1.0 + e, 1.0), (0.0, 1.0)];
        let got = coverage(&[quad], 1, 2);
        assert!((got[1] - e / 4.0).abs() < 1e-22, "{got:?}");
        assert!((got[0] - (1.0 - e / 4.0)).abs() < 1e-15, "{got:?}");
    }

    #[test]
    fn coverage_far_from_the_origin() {
        // Pixel coordinates of a large grid: a half-cell offset square.
        let (x, y) = (1.0e7, 2.0e6);
        let square = vec![
            (x + 0.5, y + 0.5),
            (x + 1.5, y + 0.5),
            (x + 1.5, y + 1.5),
            (x + 0.5, y + 1.5),
        ];
        let mut total = 0.0;
        let mut cells = 0;
        polygon_coverage(&[square], usize::MAX, usize::MAX, &mut |_, _, fractions| {
            total += fractions.iter().sum::<f64>();
            cells += fractions
                .iter()
                .filter(|f| (*f - 0.25).abs() < 1e-9)
                .count();
            ControlFlow::Continue(())
        });
        assert!((total - 1.0).abs() < 1e-9);
        assert_eq!(cells, 4);
    }
}
