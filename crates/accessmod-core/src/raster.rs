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
}
