//! Land-cover merge: stacking items by priority and finding diagonal
//! bridges, on top of the raster primitives of [`crate::raster`].
//!
//! Rules (AccessMod 5: `modules/amAnalysisMergeLandCover/amServer.R`,
//! `tools/R/amStackBarrier.R`, `tools/R/amBridgeTools.R`; the barrier rule
//! differs on purpose, see below and `docs/merge-land-cover.md`):
//!
//! - lines burn every cell they touch ([`crate::raster::line_spans`]); AccessMod 5 uses
//!   `v.to.rast -d` (densified lines);
//! - polygons burn the cells whose centre is inside
//!   ([`crate::raster::polygon_spans`]), as
//!   `v.to.rast type=area`;
//! - in the stack, the item listed first wins a cell ([`MergeCell::paint`]),
//!   as `r.patch`;
//! - a barrier empties the cells it covers in the items below it, never in
//!   those above ([`MergeCell::bar`]): roads above a river cross it, a
//!   barrier at the top blocks everything. AccessMod 5 does the same for every
//!   barrier but the first, which it applies to the whole stack;
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
    /// Priority of the highest barrier covering the cell: it empties the
    /// items of a higher rank (below it in the stack).
    pub barrier: u32,
    /// [`MergeCell::ROAD`] bit.
    pub flags: u8,
}

impl MergeCell {
    /// Rank of a cell no item covers.
    pub const NO_RANK: u32 = u32::MAX;
    /// A road item covers the cell (bridge candidate).
    pub const ROAD: u8 = 1;
    /// Cell no item covers.
    pub const EMPTY: Self = Self {
        class: 0,
        rank: Self::NO_RANK,
        barrier: Self::NO_RANK,
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

    /// A barrier at `rank` covers the cell. The result does not depend on
    /// the order barriers and items are painted in.
    #[inline]
    pub fn bar(&mut self, rank: u32) {
        self.barrier = self.barrier.min(rank);
    }

    /// Class in the merged land cover: the winning item, unless no item
    /// covers the cell or a barrier above that item empties it.
    #[inline]
    pub fn value(&self) -> Option<i32> {
        (self.rank != Self::NO_RANK && self.rank < self.barrier).then_some(self.class)
    }

    /// Whether a road item covers the cell.
    #[inline]
    pub fn is_road(&self) -> bool {
        self.flags & Self::ROAD != 0
    }

    /// Whether a barrier empties the cell: one covers it above every item
    /// that does (or no item covers it).
    #[inline]
    pub fn is_barrier(&self) -> bool {
        self.barrier < self.rank
    }
}

impl CellCodec for MergeCell {
    const BYTES: usize = 13;
    fn encode(self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.class.to_le_bytes());
        out[4..8].copy_from_slice(&self.rank.to_le_bytes());
        out[8..12].copy_from_slice(&self.barrier.to_le_bytes());
        out[12] = self.flags;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            class: i32::decode(&bytes[..4]),
            rank: u32::decode(&bytes[4..8]),
            barrier: u32::decode(&bytes[8..12]),
            flags: bytes[12],
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
        assert_eq!(MergeCell::EMPTY.value(), None);
    }

    #[test]
    fn a_barrier_empties_the_items_below_it_only() {
        // Stack: road (rank 0), river barrier (1), land cover (2).
        let mut road_over_river = MergeCell::EMPTY;
        road_over_river.bar(1);
        road_over_river.paint(1001, 0);
        road_over_river.paint(80, 2);
        assert_eq!(road_over_river.value(), Some(1001));
        assert!(!road_over_river.is_barrier());
        // The river without a road.
        let mut river = MergeCell::EMPTY;
        river.paint(80, 2);
        river.bar(1);
        assert_eq!(river.value(), None);
        assert!(river.is_barrier());
        // A barrier at the top (rank 0 here, the road then at 1) blocks it.
        let mut closed = MergeCell::EMPTY;
        closed.paint(1001, 1);
        closed.bar(3);
        closed.bar(0);
        assert_eq!(closed.value(), None);
        assert!(closed.is_barrier());
        // A barrier alone: empty, counted as a barrier.
        let mut alone = MergeCell::EMPTY;
        alone.bar(4);
        assert!(alone.is_barrier());
        assert_eq!(alone.value(), None);
        assert!(!MergeCell::EMPTY.is_barrier());
    }

    #[test]
    fn codec_round_trips() {
        let cell = MergeCell {
            class: -1_000_123,
            rank: 77,
            barrier: 12,
            flags: MergeCell::ROAD,
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
