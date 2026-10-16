//! Land-cover merge: stacking items by priority and finding diagonal
//! bridges, on top of the raster primitives of [`crate::raster`].
//!
//! Rules (AccessMod 5: `modules/amAnalysisMergeLandCover/amServer.R`,
//! `tools/R/amStackBarrier.R`, `tools/R/amBridgeTools.R`):
//!
//! - lines burn every cell they touch ([`crate::raster::line_spans`]); AccessMod 5 uses
//!   `v.to.rast -d` (densified lines);
//! - polygons burn the cells whose centre is inside
//!   ([`crate::raster::polygon_spans`]), as
//!   `v.to.rast type=area`;
//! - in the stack, the item listed first wins a cell ([`MergeCell::paint`]),
//!   as `r.patch`; barrier cells are empty whatever their position;
//! - a road cell is a bridge when a diagonal neighbour is non-empty while
//!   the two cells sharing that corner are empty ([`bridge_row`]).

use crate::raster::for_each_window3;
use crate::store::CellCodec;

/// Merge state of one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeCell {
    /// Class of the winning item; meaningless while `rank` is
    /// [`MergeCell::NO_RANK`].
    pub class: i32,
    /// Priority of the winning item (lower wins).
    pub rank: u32,
    /// [`MergeCell::ROAD`] and [`MergeCell::BARRIER`] bits.
    pub flags: u8,
}

impl MergeCell {
    /// Rank of a cell no item covers.
    pub const NO_RANK: u32 = u32::MAX;
    /// A road item covers the cell (bridge candidate).
    pub const ROAD: u8 = 1;
    /// A barrier covers the cell: it is empty in the merged land cover.
    pub const BARRIER: u8 = 2;
    /// Cell no item covers.
    pub const EMPTY: Self = Self {
        class: 0,
        rank: Self::NO_RANK,
        flags: 0,
    };

    /// Give the cell `class` if `rank` beats the current winner. The result
    /// does not depend on the order items are painted in.
    #[inline]
    pub fn paint(&mut self, class: i32, rank: u32) {
        if rank < self.rank {
            self.class = class;
            self.rank = rank;
        }
    }

    /// Class in the merged land cover: `None` for a barrier or a cell no
    /// item covers.
    #[inline]
    pub fn value(&self) -> Option<i32> {
        (self.flags & Self::BARRIER == 0 && self.rank != Self::NO_RANK).then_some(self.class)
    }

    /// Whether a road item covers the cell.
    #[inline]
    pub fn is_road(&self) -> bool {
        self.flags & Self::ROAD != 0
    }

    /// Whether a barrier covers the cell.
    #[inline]
    pub fn is_barrier(&self) -> bool {
        self.flags & Self::BARRIER != 0
    }
}

impl CellCodec for MergeCell {
    const BYTES: usize = 9;
    fn encode(self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.class.to_le_bytes());
        out[4..8].copy_from_slice(&self.rank.to_le_bytes());
        out[8] = self.flags;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            class: i32::decode(&bytes[..4]),
            rank: u32::decode(&bytes[4..8]),
            flags: bytes[8],
        }
    }
}

/// Whether the centre of a 3 × 3 window of "non-empty" flags is a diagonal
/// bridge: some diagonal neighbour is non-empty while both cells sharing
/// that corner are empty.
pub fn is_bridge(window: [[bool; 3]; 3]) -> bool {
    [(1, -1), (1, 1), (-1, 1), (-1, -1)]
        .iter()
        .any(|&(dr, dc): &(isize, isize)| {
            let r = (1 + dr) as usize;
            let c = (1 + dc) as usize;
            window[r][c] && !window[r][1] && !window[1][c]
        })
}

/// Bridges of one row: `out[c]` is true for a road cell that
/// [`is_bridge`] flags. `here` holds the non-empty flags of the row,
/// `above` and `below` those of its neighbours (`None` outside the grid,
/// where cells count as empty).
pub fn bridge_row(
    above: Option<&[bool]>,
    here: &[bool],
    below: Option<&[bool]>,
    road: &[bool],
    out: &mut [bool],
) {
    for_each_window3(above, here, below, false, |c, window| {
        out[c] = road[c] && is_bridge(window);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_keeps_the_lowest_rank_in_any_order() {
        let mut a = MergeCell::EMPTY;
        a.paint(5, 3);
        a.paint(7, 1);
        a.paint(9, 2);
        let mut b = MergeCell::EMPTY;
        b.paint(9, 2);
        b.paint(7, 1);
        b.paint(5, 3);
        assert_eq!(a, b);
        assert_eq!(a.value(), Some(7));
        a.flags |= MergeCell::BARRIER;
        assert_eq!(a.value(), None);
        assert_eq!(MergeCell::EMPTY.value(), None);
    }

    #[test]
    fn codec_round_trips() {
        let cell = MergeCell {
            class: -1_000_123,
            rank: 77,
            flags: MergeCell::ROAD | MergeCell::BARRIER,
        };
        let mut bytes = [0u8; MergeCell::BYTES];
        cell.encode(&mut bytes);
        assert_eq!(MergeCell::decode(&bytes), cell);
    }

    #[test]
    fn bridge_patterns() {
        // Non-empty diagonal, both corner neighbours empty.
        let t = true;
        let f = false;
        assert!(is_bridge([[f, f, f], [f, t, f], [f, f, t]]));
        assert!(is_bridge([[t, f, f], [f, f, f], [f, f, f]]));
        // A shared orthogonal neighbour keeps the connection.
        assert!(!is_bridge([[f, f, f], [f, t, t], [f, f, t]]));
        assert!(!is_bridge([[f, t, f], [t, t, t], [f, t, f]]));
    }

    #[test]
    fn bridge_row_treats_outside_as_empty() {
        let road = [true, true, false];
        let above = [false, false, true];
        let here = [true, false, false];
        let mut out = [false; 3];
        bridge_row(Some(&above), &here, None, &road, &mut out);
        // Column 1: diagonal above-right non-empty, above and right empty.
        assert_eq!(out, [false, true, false]);
    }
}
