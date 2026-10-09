//! Host resources: memory, disk, CPUs, and the default memory budget.

use std::path::Path;

use sysinfo::{Disks, System};

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
