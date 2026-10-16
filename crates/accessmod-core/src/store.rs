//! Tiled, disk-backed grid storage with a bounded tile cache.
//!
//! A [`TiledGrid`] splits a `rows × cols` grid into square tiles. At most
//! `capacity` tiles are resident; the least recently used one is written to a
//! scratch file when another must be loaded (same principle as the GRASS
//! segment library used by `r.walk`). When every tile fits, nothing touches
//! the disk and no file is created: one code path, two regimes.
//!
//! Storage never changes values: results are identical for any capacity.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Storage errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Scratch file could not be created, read or written.
    #[error("scratch file {path}: {source}")]
    Io {
        /// Scratch file path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Invalid geometry or capacity.
    #[error("invalid tiled grid: {0}")]
    Invalid(&'static str),
}

/// Fixed-size binary encoding of a cell value in scratch files.
pub trait CellCodec: Copy {
    /// Encoded size, bytes.
    const BYTES: usize;
    /// Write `self` into `out` (`out.len() == BYTES`).
    fn encode(self, out: &mut [u8]);
    /// Read a value from `bytes` (`bytes.len() == BYTES`).
    fn decode(bytes: &[u8]) -> Self;
}

macro_rules! numeric_codec {
    ($($t:ty),*) => {$(
        impl CellCodec for $t {
            const BYTES: usize = std::mem::size_of::<$t>();
            fn encode(self, out: &mut [u8]) {
                out.copy_from_slice(&self.to_le_bytes());
            }
            fn decode(bytes: &[u8]) -> Self {
                let mut buf = [0u8; std::mem::size_of::<$t>()];
                buf.copy_from_slice(bytes);
                <$t>::from_le_bytes(buf)
            }
        }
    )*};
}
numeric_codec!(u8, u16, u32, u64, i32, i64, f32, f64);

/// Default tile side, cells. Small tiles keep the band of tiles around the
/// Dijkstra frontier small: on a 2000² synthetic grid, 64² tiles were each
/// loaded once with 20 % of them resident, while 256² tiles thrashed at
/// 12 % (`benchmark/solver/README.md`).
pub const DEFAULT_TILE: usize = 64;

/// Number of resident tiles each grid gets from a memory budget, given the
/// bytes of one tile of every grid sharing it. The budget is split so that
/// every grid holds the same number of tiles (the solver touches the same
/// cells in all of them); at least one tile each.
pub fn tiles_for_budget(budget_bytes: u64, tile_bytes: &[usize]) -> usize {
    let per_tile_set: u64 = tile_bytes.iter().map(|&b| b as u64).sum();
    if per_tile_set == 0 {
        return 1;
    }
    usize::try_from(budget_bytes / per_tile_set)
        .unwrap_or(usize::MAX)
        .max(1)
}

/// Tiling of a `rows × cols` grid into `tile × tile` tiles. Edge tiles are
/// padded to full size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileGeometry {
    rows: usize,
    cols: usize,
    tile: usize,
    tiles_x: usize,
    tiles_y: usize,
}

impl TileGeometry {
    /// New tiling; `rows`, `cols` and `tile` must be positive.
    pub fn new(rows: usize, cols: usize, tile: usize) -> Result<Self, StoreError> {
        if rows == 0 || cols == 0 || tile == 0 {
            return Err(StoreError::Invalid(
                "rows, cols and tile size must be positive",
            ));
        }
        Ok(Self {
            rows,
            cols,
            tile,
            tiles_x: cols.div_ceil(tile),
            tiles_y: rows.div_ceil(tile),
        })
    }

    /// Grid rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Grid columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Tile side, cells.
    pub fn tile(&self) -> usize {
        self.tile
    }

    /// Number of tiles.
    pub fn n_tiles(&self) -> usize {
        self.tiles_x * self.tiles_y
    }

    /// Cells per tile, padding included.
    pub fn tile_cells(&self) -> usize {
        self.tile * self.tile
    }

    /// Tile id and offset within the tile of cell `(row, col)`.
    #[inline]
    fn locate(&self, row: usize, col: usize) -> (usize, usize) {
        if self.tile.is_power_of_two() {
            // Hot path for the default tile size: shifts and masks.
            let shift = self.tile.trailing_zeros();
            let mask = self.tile - 1;
            let (tr, r) = (row >> shift, row & mask);
            let (tc, c) = (col >> shift, col & mask);
            return (tr * self.tiles_x + tc, (r << shift) + c);
        }
        let (tr, r) = (row / self.tile, row % self.tile);
        let (tc, c) = (col / self.tile, col % self.tile);
        (tr * self.tiles_x + tc, r * self.tile + c)
    }
}

/// Cache counters, for `run.json` and benchmarks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct StoreStats {
    /// Accesses served by a resident tile.
    pub hits: u64,
    /// Accesses that loaded a tile (from disk or freshly filled).
    pub misses: u64,
    /// Tiles read back from the scratch file.
    pub tile_reads: u64,
    /// Tiles written to the scratch file.
    pub tile_writes: u64,
    /// Largest scratch file size, bytes.
    pub scratch_bytes: u64,
}

#[derive(Debug)]
struct Slot<T> {
    tile: usize,
    cells: Vec<T>,
    dirty: bool,
    last_used: u64,
}

#[derive(Debug)]
struct Scratch {
    path: PathBuf,
    file: File,
}

const NO_SLOT: u32 = u32::MAX;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Grid of `T` stored in tiles, at most `capacity` resident. See the module
/// documentation.
#[derive(Debug)]
pub struct TiledGrid<T: CellCodec> {
    geometry: TileGeometry,
    fill: T,
    capacity: usize,
    scratch_dir: PathBuf,
    name: &'static str,
    scratch: Option<Scratch>,
    on_disk: Vec<bool>,
    slot_of: Vec<u32>,
    slots: Vec<Slot<T>>,
    clock: u64,
    last: Option<(usize, usize)>,
    stats: StoreStats,
    buffer: Vec<u8>,
    /// Whether dropping this grid removes the scratch file (false for
    /// readers of a [`Frozen`] grid, which owns it).
    owns_scratch: bool,
}

impl<T: CellCodec> TiledGrid<T> {
    /// New grid with every cell set to `fill`.
    ///
    /// `capacity` is the number of resident tiles (≥ 1); `scratch_dir` holds
    /// the scratch file, created only on the first eviction and removed on
    /// drop; `name` tags the file name for diagnostics.
    pub fn new(
        geometry: TileGeometry,
        fill: T,
        capacity: usize,
        scratch_dir: &Path,
        name: &'static str,
    ) -> Result<Self, StoreError> {
        if capacity == 0 {
            return Err(StoreError::Invalid("tile capacity must be at least 1"));
        }
        if u32::try_from(geometry.n_tiles()).is_err() {
            return Err(StoreError::Invalid("too many tiles"));
        }
        let n_tiles = geometry.n_tiles();
        Ok(Self {
            geometry,
            fill,
            capacity: capacity.min(n_tiles),
            scratch_dir: scratch_dir.to_path_buf(),
            name,
            scratch: None,
            on_disk: vec![false; n_tiles],
            slot_of: vec![NO_SLOT; n_tiles],
            slots: Vec::new(),
            clock: 0,
            last: None,
            stats: StoreStats::default(),
            buffer: Vec::new(),
            owns_scratch: true,
        })
    }

    /// Tiling of this grid.
    pub fn geometry(&self) -> TileGeometry {
        self.geometry
    }

    /// Bytes of one resident tile.
    pub fn tile_bytes(geometry: TileGeometry) -> usize {
        geometry.tile_cells() * std::mem::size_of::<T>()
    }

    /// Cache counters so far.
    pub fn stats(&self) -> StoreStats {
        self.stats
    }

    /// Value of cell `(row, col)`.
    ///
    /// # Panics
    ///
    /// If the cell is outside the grid (a caller bug, not a data error).
    #[inline]
    pub fn get(&mut self, row: usize, col: usize) -> Result<T, StoreError> {
        let (slot, offset) = self.cell_slot(row, col)?;
        Ok(self.slots[slot].cells[offset])
    }

    /// Set cell `(row, col)` to `value`.
    ///
    /// # Panics
    ///
    /// If the cell is outside the grid.
    #[inline]
    pub fn set(&mut self, row: usize, col: usize, value: T) -> Result<(), StoreError> {
        let (slot, offset) = self.cell_slot(row, col)?;
        let slot = &mut self.slots[slot];
        slot.cells[offset] = value;
        slot.dirty = true;
        Ok(())
    }

    /// Copy row `row`, columns `col .. col + out.len()`, into `out`.
    ///
    /// # Panics
    ///
    /// If the span leaves the grid.
    pub fn read_row(&mut self, row: usize, col: usize, out: &mut [T]) -> Result<(), StoreError> {
        assert!(
            col + out.len() <= self.geometry.cols,
            "row span outside the grid"
        );
        let tile = self.geometry.tile;
        let mut done = 0;
        while done < out.len() {
            let c = col + done;
            let run = (tile - c % tile).min(out.len() - done);
            let (slot, offset) = self.cell_slot(row, c)?;
            out[done..done + run].copy_from_slice(&self.slots[slot].cells[offset..offset + run]);
            done += run;
        }
        Ok(())
    }

    /// Visit the grid row by row, in pieces of at most `width` columns:
    /// `visit(row, first_col, cells)`. Memory stays bounded by `width`
    /// whatever the grid width; `visit` may stop the scan with an error
    /// (cancellation).
    pub fn for_each_row_piece<E: From<StoreError>>(
        &mut self,
        width: usize,
        mut visit: impl FnMut(usize, usize, &[T]) -> Result<(), E>,
    ) -> Result<(), E> {
        let (rows, cols) = (self.geometry.rows, self.geometry.cols);
        let mut piece = vec![self.fill; width.clamp(1, cols.max(1))];
        for row in 0..rows {
            for col in (0..cols).step_by(piece.len()) {
                let n = piece.len().min(cols - col);
                self.read_row(row, col, &mut piece[..n])?;
                visit(row, col, &piece[..n])?;
            }
        }
        Ok(())
    }

    #[inline]
    fn cell_slot(&mut self, row: usize, col: usize) -> Result<(usize, usize), StoreError> {
        assert!(
            row < self.geometry.rows && col < self.geometry.cols,
            "cell ({row}, {col}) outside a {}x{} grid",
            self.geometry.rows,
            self.geometry.cols
        );
        let (tile, offset) = self.geometry.locate(row, col);
        // Fast path: same tile as the previous access. Its recency was set
        // when it became current, which is all LRU needs: no other tile
        // was used since.
        if let Some((last_tile, slot)) = self.last
            && last_tile == tile
        {
            self.stats.hits += 1;
            return Ok((slot, offset));
        }
        self.clock += 1;
        let slot = match self.slot_of[tile] {
            NO_SLOT => {
                self.stats.misses += 1;
                self.load(tile)?
            }
            slot => {
                self.stats.hits += 1;
                slot as usize
            }
        };
        self.slots[slot].last_used = self.clock;
        self.last = Some((tile, slot));
        Ok((slot, offset))
    }

    /// Make `tile` resident and return its slot, evicting the least recently
    /// used tile if the cache is full. Ties cannot occur: the clock is
    /// strictly increasing.
    fn load(&mut self, tile: usize) -> Result<usize, StoreError> {
        let slot = if self.slots.len() < self.capacity {
            self.slots.push(Slot {
                tile,
                cells: vec![self.fill; self.geometry.tile_cells()],
                dirty: false,
                last_used: self.clock,
            });
            self.slots.len() - 1
        } else {
            let victim = self
                .slots
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(i, _)| i)
                .expect("a full cache has at least one slot");
            self.evict(victim)?;
            victim
        };

        self.slots[slot].tile = tile;
        self.slots[slot].dirty = false;
        if self.on_disk[tile] {
            self.read_tile(slot)?;
        } else {
            let fill = self.fill;
            self.slots[slot].cells.fill(fill);
        }
        self.slot_of[tile] = slot as u32;
        Ok(slot)
    }

    fn evict(&mut self, slot: usize) -> Result<(), StoreError> {
        let tile = self.slots[slot].tile;
        if self.slots[slot].dirty {
            self.write_tile(slot)?;
        }
        self.slot_of[tile] = NO_SLOT;
        if self.last.is_some_and(|(t, _)| t == tile) {
            self.last = None;
        }
        Ok(())
    }

    fn tile_offset(&self, tile: usize) -> u64 {
        (tile * self.geometry.tile_cells() * T::BYTES) as u64
    }

    fn scratch(&mut self) -> Result<&mut Scratch, StoreError> {
        if self.scratch.is_none() {
            let id = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = self.scratch_dir.join(format!(
                "accessmod-{}-{id}-{}.tiles",
                std::process::id(),
                self.name
            ));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|source| StoreError::Io {
                    path: path.clone(),
                    source,
                })?;
            self.scratch = Some(Scratch { path, file });
        }
        Ok(self.scratch.as_mut().expect("scratch file created above"))
    }

    fn write_tile(&mut self, slot: usize) -> Result<(), StoreError> {
        let tile = self.slots[slot].tile;
        let offset = self.tile_offset(tile);
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.resize(self.geometry.tile_cells() * T::BYTES, 0);
        for (cell, out) in self.slots[slot]
            .cells
            .iter()
            .zip(buffer.chunks_exact_mut(T::BYTES))
        {
            cell.encode(out);
        }
        let scratch = self.scratch()?;
        let result = scratch
            .file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| scratch.file.write_all(&buffer));
        let path = scratch.path.clone();
        self.buffer = buffer;
        result.map_err(|source| StoreError::Io { path, source })?;
        self.on_disk[tile] = true;
        self.stats.tile_writes += 1;
        self.stats.scratch_bytes = self
            .stats
            .scratch_bytes
            .max(offset + (self.geometry.tile_cells() * T::BYTES) as u64);
        Ok(())
    }

    fn read_tile(&mut self, slot: usize) -> Result<(), StoreError> {
        let tile = self.slots[slot].tile;
        let offset = self.tile_offset(tile);
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.resize(self.geometry.tile_cells() * T::BYTES, 0);
        let scratch = self.scratch()?;
        let result = scratch
            .file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| scratch.file.read_exact(&mut buffer));
        let path = scratch.path.clone();
        if let Err(source) = result {
            self.buffer = buffer;
            return Err(StoreError::Io { path, source });
        }
        for (cell, bytes) in self.slots[slot]
            .cells
            .iter_mut()
            .zip(buffer.chunks_exact(T::BYTES))
        {
            *cell = T::decode(bytes);
        }
        self.buffer = buffer;
        self.stats.tile_reads += 1;
        Ok(())
    }
}

/// Read-only snapshot of a [`TiledGrid`], shared by several threads, each
/// reading through its own [`FrozenReader`]. Tiles stay in memory when the
/// grid never spilled, otherwise they are read from the grid's scratch file,
/// removed when the snapshot is dropped.
#[derive(Debug)]
pub struct Frozen<T: CellCodec> {
    geometry: TileGeometry,
    fill: T,
    scratch_dir: PathBuf,
    backend: FrozenBackend<T>,
}

#[derive(Debug)]
enum FrozenBackend<T> {
    /// Every touched tile, by tile id; `None` tiles read as the fill value.
    Memory(Vec<Option<Box<[T]>>>),
    /// Tiles in a scratch file; `on_disk[tile]` false reads as the fill value.
    File { path: PathBuf, on_disk: Vec<bool> },
}

impl<T: CellCodec> TiledGrid<T> {
    /// Freeze this grid for shared reading. Writes dirty tiles back first
    /// when the grid already uses its scratch file.
    pub fn freeze(mut self) -> Result<Frozen<T>, StoreError> {
        let backend = if self.scratch.is_none() {
            let mut tiles: Vec<Option<Box<[T]>>> = Vec::new();
            tiles.resize_with(self.geometry.n_tiles(), || None);
            for slot in std::mem::take(&mut self.slots) {
                tiles[slot.tile] = Some(slot.cells.into_boxed_slice());
            }
            FrozenBackend::Memory(tiles)
        } else {
            for slot in 0..self.slots.len() {
                if self.slots[slot].dirty {
                    self.write_tile(slot)?;
                    self.slots[slot].dirty = false;
                }
            }
            if let Some(scratch) = &mut self.scratch {
                scratch.file.flush().map_err(|source| StoreError::Io {
                    path: scratch.path.clone(),
                    source,
                })?;
            }
            self.owns_scratch = false;
            let path = self
                .scratch
                .as_ref()
                .map(|s| s.path.clone())
                .expect("checked: the grid has a scratch file");
            FrozenBackend::File {
                path,
                on_disk: std::mem::take(&mut self.on_disk),
            }
        };
        Ok(Frozen {
            geometry: self.geometry,
            fill: self.fill,
            scratch_dir: self.scratch_dir.clone(),
            backend,
        })
    }
}

impl<T: CellCodec> Frozen<T> {
    /// Tiling of the grid.
    pub fn geometry(&self) -> TileGeometry {
        self.geometry
    }

    /// Whether tiles are shared in memory (no per-reader cache needed).
    pub fn in_memory(&self) -> bool {
        matches!(self.backend, FrozenBackend::Memory(_))
    }

    /// A reader with its own cache of `capacity` tiles (file backend only).
    pub fn reader(&self, capacity: usize) -> Result<FrozenReader<'_, T>, StoreError> {
        match &self.backend {
            FrozenBackend::Memory(tiles) => Ok(FrozenReader::Memory {
                geometry: self.geometry,
                fill: self.fill,
                tiles,
            }),
            FrozenBackend::File { path, on_disk } => {
                let mut grid = TiledGrid::new(
                    self.geometry,
                    self.fill,
                    capacity,
                    &self.scratch_dir,
                    "reader",
                )?;
                let file =
                    OpenOptions::new()
                        .read(true)
                        .open(path)
                        .map_err(|source| StoreError::Io {
                            path: path.clone(),
                            source,
                        })?;
                grid.scratch = Some(Scratch {
                    path: path.clone(),
                    file,
                });
                grid.on_disk = on_disk.clone();
                grid.owns_scratch = false;
                Ok(FrozenReader::File(Box::new(grid)))
            }
        }
    }
}

impl<T: CellCodec> Drop for Frozen<T> {
    fn drop(&mut self) {
        if let FrozenBackend::File { path, .. } = &self.backend {
            // Best effort, as for TiledGrid.
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Reader of a [`Frozen`] grid; not shared between threads.
#[derive(Debug)]
pub enum FrozenReader<'a, T: CellCodec> {
    /// Tiles shared in memory.
    Memory {
        /// Tiling.
        geometry: TileGeometry,
        /// Value of tiles never touched.
        fill: T,
        /// Tiles by id.
        tiles: &'a [Option<Box<[T]>>],
    },
    /// Private tile cache over the shared scratch file.
    File(Box<TiledGrid<T>>),
}

impl<T: CellCodec> FrozenReader<'_, T> {
    /// Tiling of the grid.
    pub fn geometry(&self) -> TileGeometry {
        match self {
            Self::Memory { geometry, .. } => *geometry,
            Self::File(grid) => grid.geometry(),
        }
    }

    /// Value of cell `(row, col)`.
    ///
    /// # Panics
    ///
    /// If the cell is outside the grid.
    #[inline]
    pub fn get(&mut self, row: usize, col: usize) -> Result<T, StoreError> {
        match self {
            Self::Memory {
                geometry,
                fill,
                tiles,
            } => {
                assert!(
                    row < geometry.rows && col < geometry.cols,
                    "cell outside the grid"
                );
                let (tile, offset) = geometry.locate(row, col);
                Ok(tiles[tile].as_ref().map_or(*fill, |cells| cells[offset]))
            }
            Self::File(grid) => grid.get(row, col),
        }
    }

    /// Cache counters (zero for the memory backend).
    pub fn stats(&self) -> StoreStats {
        match self {
            Self::Memory { .. } => StoreStats::default(),
            Self::File(grid) => grid.stats(),
        }
    }
}

impl<T: CellCodec> Drop for TiledGrid<T> {
    fn drop(&mut self) {
        if let Some(scratch) = self.scratch.take()
            && self.owns_scratch
        {
            drop(scratch.file);
            // Best effort: a leftover scratch file is harmless and lives in
            // the user's temporary directory.
            let _ = std::fs::remove_file(&scratch.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("accessmod-store-tests");
        std::fs::create_dir_all(&dir).expect("temp dir is writable");
        dir
    }

    #[test]
    fn budget_split() {
        assert_eq!(tiles_for_budget(1000, &[100, 150]), 4);
        assert_eq!(tiles_for_budget(10, &[100, 150]), 1);
        assert_eq!(tiles_for_budget(10, &[]), 1);
    }

    #[test]
    fn geometry_pads_edge_tiles() {
        let g = TileGeometry::new(5, 7, 4).expect("valid");
        assert_eq!(g.n_tiles(), 4);
        assert_eq!(g.locate(0, 0), (0, 0));
        assert_eq!(g.locate(4, 6), (3, 2));
        assert!(TileGeometry::new(0, 1, 1).is_err());
    }

    #[test]
    fn fast_path_never_creates_a_file() {
        let g = TileGeometry::new(10, 10, 4).expect("valid");
        let mut grid = TiledGrid::new(g, 0u32, g.n_tiles(), &scratch_dir(), "fast").expect("new");
        for r in 0..10 {
            for c in 0..10 {
                grid.set(r, c, (r * 10 + c) as u32).expect("set");
            }
        }
        assert!(grid.scratch.is_none());
        assert_eq!(grid.get(9, 9).expect("get"), 99);
        assert_eq!(grid.stats().tile_writes, 0);
    }

    #[test]
    fn eviction_round_trips_and_cleans_up() {
        let g = TileGeometry::new(9, 9, 2).expect("valid");
        let mut grid = TiledGrid::new(g, f64::NAN, 1, &scratch_dir(), "evict").expect("new");
        for r in 0..9 {
            for c in 0..9 {
                grid.set(r, c, (r * 100 + c) as f64 + 0.25).expect("set");
            }
        }
        for r in (0..9).rev() {
            for c in (0..9).rev() {
                assert_eq!(grid.get(r, c).expect("get"), (r * 100 + c) as f64 + 0.25);
            }
        }
        let mut row = [0.0; 7];
        grid.read_row(4, 1, &mut row).expect("row");
        assert_eq!(
            row,
            [401.25, 402.25, 403.25, 404.25, 405.25, 406.25, 407.25]
        );
        let path = grid
            .scratch
            .as_ref()
            .map(|s| s.path.clone())
            .expect("spilled");
        assert!(path.exists());
        assert!(grid.stats().tile_writes > 0 && grid.stats().tile_reads > 0);
        drop(grid);
        assert!(!path.exists());
    }

    #[test]
    fn untouched_cells_read_as_fill() {
        let g = TileGeometry::new(6, 6, 2).expect("valid");
        let mut grid = TiledGrid::new(g, 7u16, 1, &scratch_dir(), "fill").expect("new");
        grid.set(0, 0, 1).expect("set");
        assert_eq!(grid.get(5, 5).expect("get"), 7);
        assert_eq!(grid.get(0, 0).expect("get"), 1);
        assert_eq!(grid.get(0, 1).expect("get"), 7);
    }

    proptest! {
        #[test]
        fn matches_a_plain_vector_for_any_capacity(
            rows in 1usize..20, cols in 1usize..20, tile in 1usize..6, capacity in 1usize..5,
            ops in proptest::collection::vec((0usize..400, any::<u32>(), any::<bool>()), 1..300),
        ) {
            let g = TileGeometry::new(rows, cols, tile).expect("valid");
            let mut grid = TiledGrid::new(g, 0u32, capacity, &scratch_dir(), "prop").expect("new");
            let mut reference = vec![0u32; rows * cols];
            for (cell, value, write) in ops {
                let i = cell % (rows * cols);
                let (r, c) = (i / cols, i % cols);
                if write {
                    grid.set(r, c, value).expect("set");
                    reference[i] = value;
                } else {
                    prop_assert_eq!(grid.get(r, c).expect("get"), reference[i]);
                }
            }
            for (i, expected) in reference.iter().enumerate() {
                prop_assert_eq!(grid.get(i / cols, i % cols).expect("get"), *expected);
            }
        }
    }
}
