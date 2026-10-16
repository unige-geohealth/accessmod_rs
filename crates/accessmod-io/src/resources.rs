//! Host resources: memory, disk, CPUs, and the default memory budget.

use std::path::{Path, PathBuf};

use accessmod_core::store::{DEFAULT_TILE, TileGeometry, tiles_for_budget};
use serde::Serialize;

use sysinfo::{Disks, System};

use crate::error::{Error, Result};

/// Smallest default memory budget, bytes.
pub const MIN_DEFAULT_BUDGET: u64 = 256 << 20;
/// Fraction of the available memory used by default.
pub const DEFAULT_BUDGET_FRACTION: f64 = 0.5;

/// Memory available to new allocations, bytes.
pub fn available_memory() -> u64 {
    let mut system = System::new();
    system.refresh_memory();
    system.available_memory()
}

/// Total physical memory, bytes.
pub fn total_memory() -> u64 {
    let mut system = System::new();
    system.refresh_memory();
    system.total_memory()
}

/// Default `--memory`: half of the available memory, at least
/// [`MIN_DEFAULT_BUDGET`].
pub fn default_memory_budget() -> u64 {
    ((available_memory() as f64 * DEFAULT_BUDGET_FRACTION) as u64).max(MIN_DEFAULT_BUDGET)
}

/// Free space of the filesystem holding `path`, bytes, if known.
pub fn free_disk(path: &Path) -> Option<u64> {
    let path = path.canonicalize().ok()?;
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// Logical CPUs available to this process.
pub fn cpus() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Default `--threads`: available CPUs, capped at 16.
pub fn default_threads() -> usize {
    cpus().min(16)
}

/// Parse a memory size such as `512M`, `2G`, `1.5GiB` or `1073741824`.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number.parse().ok()?;
    let factor: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        _ => return None,
    };
    let bytes = number * factor as f64;
    (bytes.is_finite() && bytes >= 1.0).then_some(bytes as u64)
}

/// Peak resident memory of this process, bytes, where the platform reports
/// it.
#[cfg(unix)]
pub fn peak_rss() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` fills the struct it is given; RUSAGE_SELF is valid
    // and the pointer is to a properly sized, writable `rusage`.
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: `getrusage` returned 0, so the struct is initialised.
    let max_rss = unsafe { usage.assume_init() }.ru_maxrss;
    let max_rss = u64::try_from(max_rss).ok()?;
    // macOS reports bytes, other Unix systems kilobytes.
    Some(if cfg!(target_os = "macos") {
        max_rss
    } else {
        max_rss * 1024
    })
}

/// Peak resident memory of this process, bytes, where the platform reports
/// it.
#[cfg(not(unix))]
pub fn peak_rss() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("512M"), Some(512 << 20));
        assert_eq!(parse_size("2G"), Some(2 << 30));
        assert_eq!(parse_size("1.5GiB"), Some(3 << 29));
        assert_eq!(parse_size("2 gb"), Some(2 << 30));
        assert_eq!(parse_size("x"), None);
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("5Q"), None);
    }

    #[test]
    fn host_figures_are_plausible() {
        assert!(total_memory() > 0);
        assert!(cpus() >= 1);
        assert!(default_memory_budget() >= MIN_DEFAULT_BUDGET);
        assert!(free_disk(&std::env::temp_dir()).is_some_and(|b| b > 0));
        assert!(peak_rss().is_none_or(|b| b > 0));
    }
}

/// Execution settings: they change speed, never results, and are not part
/// of the job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionSettings {
    /// Memory budget, bytes.
    pub memory_bytes: u64,
    /// Threads for GDAL encoding and compression.
    pub threads: usize,
    /// Directory for scratch tiles.
    pub tmp_dir: PathBuf,
}

impl ExecutionSettings {
    /// Defaults from the host: half the available memory, available CPUs,
    /// the system temporary directory.
    pub fn from_host() -> Self {
        Self {
            memory_bytes: default_memory_budget(),
            threads: default_threads(),
            tmp_dir: std::env::temp_dir(),
        }
    }
}

/// Memory and disk plan of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourcePlan {
    /// Memory budget, bytes.
    pub memory_budget_bytes: u64,
    /// Memory reserved for fixed buffers (GDAL cache, output strips), bytes.
    pub fixed_bytes: u64,
    /// Memory reserved for the priority queue, bytes.
    pub queue_reserve_bytes: u64,
    /// GDAL block cache (`GDAL_CACHEMAX`), bytes.
    pub gdal_cache_bytes: u64,
    /// Tile side, cells.
    pub tile: usize,
    /// Tiles per grid.
    pub total_tiles: usize,
    /// Resident tiles per grid.
    pub resident_tiles: usize,
    /// Memory to keep every tile resident (no scratch I/O), bytes.
    pub in_memory_bytes: u64,
    /// Smallest budget accepted, bytes.
    pub minimum_bytes: u64,
    /// Scratch disk needed in the worst case, bytes (0 when every tile is
    /// resident).
    pub scratch_bytes: u64,
    /// Free space where scratch files go, bytes, if known.
    pub free_disk_bytes: Option<u64>,
}

impl ResourcePlan {
    /// Size GDAL's block cache (`GDAL_CACHEMAX`) as planned.
    pub fn configure_gdal(&self) -> Result<()> {
        gdal::config::set_config_option("GDAL_CACHEMAX", &self.gdal_cache_bytes.to_string())
            .map_err(Error::gdal("GDAL_CACHEMAX"))
    }

    /// Plan for tiled grids taking `per_tile` bytes per tile position, with
    /// `buffer_bytes` of fixed buffers and `queue_tenths` tenths of the
    /// budget reserved for a priority queue (0 when there is none).
    pub fn for_tiles(
        rows: usize,
        cols: usize,
        settings: &ExecutionSettings,
        per_tile: u64,
        buffer_bytes: u64,
        queue_tenths: u64,
    ) -> Result<Self> {
        let geometry = TileGeometry::new(rows, cols, DEFAULT_TILE)?;
        let total_tiles = geometry.n_tiles();
        let budget = settings.memory_bytes;
        let gdal_cache_bytes = (budget / 8).clamp(16 << 20, 512 << 20);
        let fixed_bytes = gdal_cache_bytes + buffer_bytes;
        let queue_reserve_bytes = budget / 10 * queue_tenths;
        // The solver needs at least the band of tiles around its frontier;
        // two tile rows and two tile columns across the grid is the floor.
        let tiles_x = cols.div_ceil(DEFAULT_TILE);
        let tiles_y = rows.div_ceil(DEFAULT_TILE);
        let band_tiles = (2 * (tiles_x + tiles_y)).min(total_tiles) as u64;
        // The queue reserve is q tenths of the budget B, so tiles get
        // (1 − q/10)·B − fixed: B must be at least (fixed + tiles) / (1 − q/10).
        let with_reserve =
            |tile_bytes: u64| (fixed_bytes + tile_bytes).div_ceil(10 - queue_tenths) * 10;
        let minimum_bytes = with_reserve(band_tiles * per_tile);
        let in_memory_bytes = with_reserve(total_tiles as u64 * per_tile);
        let tile_budget = budget.saturating_sub(fixed_bytes + queue_reserve_bytes);
        let resident_tiles = tiles_for_budget(tile_budget, &[per_tile as usize]).min(total_tiles);
        let scratch_bytes = if resident_tiles >= total_tiles {
            0
        } else {
            total_tiles as u64 * per_tile
        };
        let free_disk_bytes = free_disk(&settings.tmp_dir);
        let plan = Self {
            memory_budget_bytes: budget,
            fixed_bytes,
            queue_reserve_bytes,
            gdal_cache_bytes,
            tile: DEFAULT_TILE,
            total_tiles,
            resident_tiles,
            in_memory_bytes,
            minimum_bytes,
            scratch_bytes,
            free_disk_bytes,
        };
        let mut issues = Vec::new();
        if budget < minimum_bytes {
            issues.push(format!(
                "memory budget {} is below the minimum {} for a {rows} x {cols} grid \
                 ({} keeps everything in memory); raise --memory",
                mib(budget),
                mib(minimum_bytes),
                mib(in_memory_bytes)
            ));
        }
        if let Some(free) = free_disk_bytes
            && free < scratch_bytes
        {
            issues.push(format!(
                "scratch directory {} has {} free, the run may need {}; use --tmp-dir \
                 or raise --memory (in memory: {})",
                settings.tmp_dir.display(),
                mib(free),
                mib(scratch_bytes),
                mib(in_memory_bytes)
            ));
        }
        if issues.is_empty() {
            Ok(plan)
        } else {
            Err(Error::Validation { issues })
        }
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.0} MiB", bytes as f64 / (1u64 << 20) as f64)
}
