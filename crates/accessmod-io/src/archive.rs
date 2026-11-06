//! Zip archives with a manifest: data archives (Data › Archive) and project
//! files (`.am6p`).
//!
//! [`write`] streams files from disk into a zip, never holding one in
//! memory, and ends it with `manifest.json`: format, versions, the caller's
//! description and, per file, its size and SHA-256. Already-compressed
//! files (GeoTIFF with DEFLATE, PNG, zip) are stored, the others deflated.
//! Large files use zip64.
//!
//! [`extract`] treats the zip as hostile: names must be relative paths
//! without `..`, links are refused, the entry count and the bytes written
//! are bounded whatever the headers claim (zip bombs), every file must be
//! listed in the manifest with the right size and SHA-256, and nothing is
//! written outside the target directory. It returns errors, never panics.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::analysis::Monitor;
use crate::error::{Error, Result};

/// Progress stage of [`write`]: bytes read of the files to archive.
pub const STAGE_WRITE: &str = "archive";
/// Progress stage of [`extract`]: bytes extracted of the archive.
pub const STAGE_EXTRACT: &str = "extract";

/// Name of the manifest inside an archive.
pub const MANIFEST: &str = "manifest.json";

/// Version of the manifest layout.
pub const MANIFEST_VERSION: u32 = 1;

/// A file to put in an archive.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path inside the archive: relative, `/`-separated, no `..`.
    pub name: String,
    /// Content.
    pub source: Source,
}

/// Content of an [`Entry`].
#[derive(Debug, Clone)]
pub enum Source {
    /// A file, streamed.
    File(PathBuf),
    /// Small content made by the caller (a job, a description).
    Bytes(Vec<u8>),
}

/// A file listed in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ManifestFile {
    /// Path inside the archive.
    pub path: String,
    /// Size, bytes.
    pub bytes: u64,
    /// SHA-256 of the content, hexadecimal.
    pub sha256: String,
}

/// `manifest.json`: what an archive holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    /// Kind of archive, e.g. `accessmod-data` or `am6p`.
    pub format: String,
    /// [`MANIFEST_VERSION`] of the writer.
    pub version: u32,
    /// Version of the engine that wrote it.
    pub accessmod: String,
    /// Creation, seconds since the Unix epoch.
    pub created: i64,
    /// The writer's description of the content (project, datasets, runs).
    pub content: serde_json::Value,
    /// Every file but the manifest, in archive order.
    pub files: Vec<ManifestFile>,
}

/// Bounds of an extraction.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Most entries, directories included.
    pub entries: usize,
    /// Most bytes written, all files together.
    pub bytes: u64,
    /// Largest manifest, bytes.
    pub manifest_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            entries: 100_000,
            bytes: 1 << 40,
            manifest_bytes: 64 << 20,
        }
    }
}

fn zip_error(context: &str) -> impl Fn(zip::result::ZipError) -> Error + '_ {
    move |e| Error::invalid(format!("{context}: {e}"))
}

/// Whether `name` is a safe archive path: relative, `/`-separated normal
/// components, none empty, `.` or `..`.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !name.contains(':')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Files already compressed gain nothing from DEFLATE.
fn method_for(name: &str) -> CompressionMethod {
    let lower = name.to_ascii_lowercase();
    if [".tif", ".tiff", ".png", ".zip", ".am6p", ".gz"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    }
}

/// `YYYY-MM-DD_HHMMSS`, UTC, of `secs` since the Unix epoch: archive
/// names that sort by time.
pub fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}{:02}{:02}",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

/// Hex SHA-256.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write `entries` and their manifest to a zip in `dir`, then publish it as
/// the first of `names` (a sequence of file names) not taken yet. The zip
/// is written to a temporary file of its own (`.<…>.partial`) and linked
/// to its name without replacing anything: an interrupted or failed write
/// leaves no archive, and concurrent writers never share a file or a name.
/// `format` and `content` go to the manifest.
pub fn write(
    dir: &Path,
    names: &mut dyn Iterator<Item = String>,
    entries: &[Entry],
    format: &str,
    content: serde_json::Value,
    monitor: Monitor<'_>,
) -> Result<(PathBuf, Manifest)> {
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        if !safe_name(&entry.name) || entry.name == MANIFEST {
            return Err(Error::invalid(format!(
                "{:?} cannot be a path inside an archive",
                entry.name
            )));
        }
        if !seen.insert(entry.name.as_str()) {
            return Err(Error::invalid(format!(
                "{} is twice in the archive",
                entry.name
            )));
        }
    }
    let partial = partial_file(dir)?;
    let result = write_to(&partial, entries, format, content, monitor)
        .and_then(|manifest| publish(&partial, dir, names).map(|path| (path, manifest)));
    let _ = std::fs::remove_file(&partial);
    result
}

/// A new, empty temporary file in `dir`, owned by this writer.
fn partial_file(dir: &Path) -> Result<PathBuf> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    for _ in 0..100 {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!(".{}-{nanos}-{n}.partial", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::file(&path)(e)),
        }
    }
    Err(Error::invalid(format!(
        "{}: no free temporary file name",
        dir.display()
    )))
}

/// Link `partial` to the first free name of `names` in `dir`; a hard link
/// never replaces an existing file.
fn publish(partial: &Path, dir: &Path, names: &mut dyn Iterator<Item = String>) -> Result<PathBuf> {
    for name in names.take(1000) {
        if !safe_name(&name) || name.contains('/') {
            return Err(Error::invalid(format!("{name:?} cannot name an archive")));
        }
        let path = dir.join(&name);
        match std::fs::hard_link(partial, &path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::file(&path)(e)),
        }
    }
    Err(Error::invalid(format!(
        "{}: no free archive name",
        dir.display()
    )))
}

fn write_to(
    path: &Path,
    entries: &[Entry],
    format: &str,
    content: serde_json::Value,
    monitor: Monitor<'_>,
) -> Result<Manifest> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(Error::file(path))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let mut files = Vec::with_capacity(entries.len());
    let mut buffer = vec![0u8; 1 << 20];
    // Progress in bytes: the files as they are now (one may still grow).
    let total: u64 = entries
        .iter()
        .map(|e| match &e.source {
            Source::File(path) => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            Source::Bytes(data) => data.len() as u64,
        })
        .fold(0u64, u64::saturating_add);
    let mut done = 0u64;
    monitor.advance(STAGE_WRITE, 0, Some(total));
    for entry in entries {
        // A streamed file may grow while it is read: always zip64.
        let options = SimpleFileOptions::default()
            .compression_method(method_for(&entry.name))
            .large_file(match &entry.source {
                Source::File(_) => true,
                Source::Bytes(b) => b.len() as u64 >= u32::MAX as u64,
            });
        zip.start_file(entry.name.as_str(), options)
            .map_err(zip_error(path.to_string_lossy().as_ref()))?;
        let mut hasher = Sha256::new();
        let mut bytes = 0u64;
        match &entry.source {
            Source::Bytes(data) => {
                hasher.update(data);
                zip.write_all(data).map_err(Error::file(path))?;
                bytes = data.len() as u64;
                done += bytes;
            }
            Source::File(source) => {
                let mut input = std::fs::File::open(source).map_err(Error::file(source))?;
                loop {
                    if monitor.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let n = input.read(&mut buffer).map_err(Error::file(source))?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                    zip.write_all(&buffer[..n]).map_err(Error::file(path))?;
                    bytes += n as u64;
                    done += n as u64;
                    monitor.advance(STAGE_WRITE, done, Some(total.max(done)));
                }
            }
        }
        files.push(ManifestFile {
            path: entry.name.clone(),
            bytes,
            sha256: hex(&hasher.finalize()),
        });
    }
    let manifest = Manifest {
        format: format.to_owned(),
        version: MANIFEST_VERSION,
        accessmod: env!("CARGO_PKG_VERSION").to_owned(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        content,
        files,
    };
    let text = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| Error::invalid(format!("manifest: {e}")))?;
    zip.start_file(
        MANIFEST,
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )
    .map_err(zip_error(path.to_string_lossy().as_ref()))?;
    zip.write_all(&text).map_err(Error::file(path))?;
    let writer = zip
        .finish()
        .map_err(zip_error(path.to_string_lossy().as_ref()))?;
    let file = writer
        .into_inner()
        .map_err(|e| Error::invalid(format!("{}: {}", path.display(), e.error())))?;
    file.sync_all().map_err(Error::file(path))?;
    Ok(manifest)
}

/// Read the manifest of the zip `archive` without extracting it.
pub fn manifest(archive: &Path, limits: Limits) -> Result<Manifest> {
    let mut zip = open(archive, limits)?;
    read_manifest(&mut zip, archive, limits)
}

/// Open the zip `archive` once its entry count, read from its end record,
/// is within `limits` (the zip library indexes every entry in memory), and
/// check that no two entries share a name (the library keeps only the last).
fn open(archive: &Path, limits: Limits) -> Result<zip::ZipArchive<std::fs::File>> {
    let name = archive.to_string_lossy();
    let mut file = std::fs::File::open(archive).map_err(Error::file(archive))?;
    let declared = central_entries(&mut file)
        .map_err(|e| Error::invalid(format!("{name}: not a zip archive ({e})")))?;
    if declared > limits.entries as u64 {
        return Err(Error::invalid(format!(
            "{name} holds {declared} entries; at most {} are read",
            limits.entries
        )));
    }
    let zip = zip::ZipArchive::new(file).map_err(zip_error(&name))?;
    if zip.len() as u64 != declared {
        return Err(Error::invalid(format!(
            "{name}: entries share a name, or the directory is damaged"
        )));
    }
    Ok(zip)
}

/// Entry count of a zip from its end of central directory record (zip64
/// when the count overflows 16 bits).
fn central_entries(file: &mut std::fs::File) -> std::io::Result<u64> {
    use std::io::{Seek, SeekFrom};
    let invalid =
        |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_owned());
    let len = file.seek(SeekFrom::End(0))?;
    let tail = len.min(22 + 65_535);
    file.seek(SeekFrom::Start(len - tail))?;
    let mut buffer = vec![0u8; tail as usize];
    file.read_exact(&mut buffer)?;
    let at = (0..buffer.len().saturating_sub(21))
        .rev()
        .find(|&i| buffer[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| invalid("no end of central directory"))?;
    let count = u16::from_le_bytes([buffer[at + 10], buffer[at + 11]]);
    if count != u16::MAX {
        return Ok(u64::from(count));
    }
    // Zip64: the locator sits just before the end record.
    let locator = (len - tail + at as u64)
        .checked_sub(20)
        .ok_or_else(|| invalid("no zip64 locator"))?;
    let mut bytes = [0u8; 20];
    file.seek(SeekFrom::Start(locator))?;
    file.read_exact(&mut bytes)?;
    if bytes[..4] != [0x50, 0x4b, 0x06, 0x07] {
        return Err(invalid("no zip64 locator"));
    }
    let record = u64::from_le_bytes(bytes[8..16].try_into().map_err(|_| invalid("zip64"))?);
    let mut bytes = [0u8; 40];
    file.seek(SeekFrom::Start(record))?;
    file.read_exact(&mut bytes)?;
    if bytes[..4] != [0x50, 0x4b, 0x06, 0x06] {
        return Err(invalid("no zip64 end record"));
    }
    Ok(u64::from_le_bytes(
        bytes[32..40].try_into().map_err(|_| invalid("zip64"))?,
    ))
}

fn read_manifest<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    archive: &Path,
    limits: Limits,
) -> Result<Manifest> {
    let entry = zip
        .by_name(MANIFEST)
        .map_err(|_| Error::invalid(format!("{} has no {MANIFEST}", archive.display())))?;
    let mut text = Vec::new();
    entry
        .take(limits.manifest_bytes + 1)
        .read_to_end(&mut text)
        .map_err(Error::file(archive))?;
    if text.len() as u64 > limits.manifest_bytes {
        return Err(Error::invalid(format!(
            "{}: {MANIFEST} is too large",
            archive.display()
        )));
    }
    let manifest: Manifest = serde_json::from_slice(&text)
        .map_err(|e| Error::invalid(format!("{}: {MANIFEST}: {e}", archive.display())))?;
    if manifest.version > MANIFEST_VERSION {
        return Err(Error::invalid(format!(
            "{} was written by a newer accessmod (manifest version {}): update accessmod",
            archive.display(),
            manifest.version
        )));
    }
    Ok(manifest)
}

/// Extract the zip `archive` into `target`, a directory that must not
/// exist yet, and return its manifest. On any error `target` is removed:
/// either every file listed is there, checked, or nothing is.
pub fn extract(
    archive: &Path,
    target: &Path,
    limits: Limits,
    monitor: Monitor<'_>,
) -> Result<Manifest> {
    if target.exists() {
        return Err(Error::invalid(format!(
            "{} already exists",
            target.display()
        )));
    }
    std::fs::create_dir_all(target).map_err(Error::file(target))?;
    let result = extract_into(archive, target, limits, monitor);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(target);
    }
    result
}

fn extract_into(
    archive: &Path,
    target: &Path,
    limits: Limits,
    monitor: Monitor<'_>,
) -> Result<Manifest> {
    let name = archive.to_string_lossy();
    let mut zip = open(archive, limits)?;
    let manifest = read_manifest(&mut zip, archive, limits)?;
    let listed: std::collections::HashMap<&str, &ManifestFile> = manifest
        .files
        .iter()
        .map(|f| (f.path.as_str(), f))
        .collect();
    if listed.len() != manifest.files.len() {
        return Err(Error::invalid(format!(
            "{name}: {MANIFEST} lists a file twice"
        )));
    }
    let mut found = std::collections::HashSet::new();
    let mut written = 0u64;
    // Hostile sizes: never an overflow (each is checked against the limits below).
    let total = manifest
        .files
        .iter()
        .fold(0u64, |sum, f| sum.saturating_add(f.bytes));
    monitor.advance(STAGE_EXTRACT, 0, Some(total));
    let mut buffer = vec![0u8; 1 << 20];
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(zip_error(&name))?;
        let entry_name = entry.name().map_err(zip_error(&name))?.into_owned();
        if entry_name == MANIFEST {
            continue;
        }
        if entry.is_symlink() {
            return Err(Error::invalid(format!(
                "{name}: {entry_name} is a link; links are refused"
            )));
        }
        let trimmed = entry_name.strip_suffix('/').unwrap_or(&entry_name);
        if !safe_name(trimmed) {
            return Err(Error::invalid(format!(
                "{name}: {entry_name:?} leaves the archive directory"
            )));
        }
        if entry.is_dir() {
            continue;
        }
        let Some(expected) = listed.get(trimmed) else {
            return Err(Error::invalid(format!(
                "{name}: {entry_name} is not in {MANIFEST}"
            )));
        };
        if !found.insert(trimmed.to_owned()) {
            return Err(Error::invalid(format!("{name}: {entry_name} is twice")));
        }
        if expected.bytes > limits.bytes.saturating_sub(written) {
            return Err(Error::invalid(format!(
                "{name}: more than {} bytes to extract",
                limits.bytes
            )));
        }
        let mut path = target.to_path_buf();
        for part in trimmed.split('/') {
            path.push(part);
        }
        // Belt and braces: the joined path stays below the target.
        if path
            .strip_prefix(target)
            .map(|rest| {
                rest.components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            })
            .unwrap_or(true)
        {
            return Err(Error::invalid(format!(
                "{name}: {entry_name:?} leaves the archive directory"
            )));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::file(parent))?;
        }
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(Error::file(&path))?;
        let mut hasher = Sha256::new();
        let mut bytes = 0u64;
        // Never more than the manifest says, whatever the header claims.
        let mut limited = (&mut entry).take(expected.bytes + 1);
        loop {
            if monitor.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let n = limited
                .read(&mut buffer)
                .map_err(|e| Error::invalid(format!("{name}: {entry_name}: {e}")))?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > expected.bytes {
                return Err(Error::invalid(format!(
                    "{name}: {entry_name} is larger than {MANIFEST} says"
                )));
            }
            hasher.update(&buffer[..n]);
            output.write_all(&buffer[..n]).map_err(Error::file(&path))?;
            monitor.advance(STAGE_EXTRACT, written + bytes, Some(total));
        }
        written += bytes;
        if bytes != expected.bytes || hex(&hasher.finalize()) != expected.sha256 {
            return Err(Error::invalid(format!(
                "{name}: {entry_name} does not match {MANIFEST} (damaged archive)"
            )));
        }
    }
    if let Some(missing) = manifest
        .files
        .iter()
        .find(|f| !found.contains(f.path.as_str()))
    {
        return Err(Error::invalid(format!(
            "{name}: {} is in {MANIFEST} but not in the archive",
            missing.path
        )));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> impl Iterator<Item = String> {
        std::iter::once(name.to_owned())
    }

    fn sample(dir: &Path) -> (PathBuf, Manifest) {
        let data = dir.join("data.tif");
        std::fs::write(&data, vec![7u8; 3 << 20]).unwrap();
        write(
            dir,
            &mut named("a.zip"),
            &[
                Entry {
                    name: "datasets/1.tif".into(),
                    source: Source::File(data),
                },
                Entry {
                    name: "jobs/1.json".into(),
                    source: Source::Bytes(b"{\"a\": 1}".to_vec()),
                },
            ],
            "test",
            serde_json::json!({"note": "x"}),
            Monitor::QUIET,
        )
        .unwrap()
    }

    /// A zip written by hand, entry by entry, with `manifest` as manifest.
    fn raw(dir: &Path, entries: &[(&str, &[u8])], manifest: &Manifest) -> PathBuf {
        let path = dir.join("raw.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, data) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.start_file(MANIFEST, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(manifest).unwrap())
            .unwrap();
        zip.finish().unwrap();
        path
    }

    fn listing(files: &[(&str, &[u8])]) -> Manifest {
        Manifest {
            format: "test".into(),
            version: MANIFEST_VERSION,
            accessmod: "0".into(),
            created: 0,
            content: serde_json::Value::Null,
            files: files
                .iter()
                .map(|(path, data)| ManifestFile {
                    path: (*path).into(),
                    bytes: data.len() as u64,
                    sha256: hex(&Sha256::digest(data)),
                })
                .collect(),
        }
    }

    #[test]
    fn stamps_are_utc_dates() {
        assert_eq!(utc_stamp(0), "1970-01-01_000000");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29_000000");
        assert_eq!(utc_stamp(1_793_975_525), "2026-11-06_143205");
    }

    #[test]
    fn round_trip_checks_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let (archive, written) = sample(dir.path());
        assert_eq!(archive, dir.path().join("a.zip"));
        assert_eq!(written.files.len(), 2);
        // No temporary file left.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        // The name is taken: the next one of the sequence is used, nothing replaced.
        let before = std::fs::read(&archive).unwrap();
        let (second, _) = write(
            dir.path(),
            &mut ["a.zip", "a_2.zip"].into_iter().map(String::from),
            &[],
            "t",
            serde_json::Value::Null,
            Monitor::QUIET,
        )
        .unwrap();
        assert_eq!(second, dir.path().join("a_2.zip"));
        assert_eq!(std::fs::read(&archive).unwrap(), before);
        assert_eq!(manifest(&archive, Limits::default()).unwrap(), written);
        let out = dir.path().join("out");
        let read = extract(&archive, &out, Limits::default(), Monitor::QUIET).unwrap();
        assert_eq!(read, written);
        assert_eq!(
            std::fs::read(out.join("datasets/1.tif")).unwrap(),
            vec![7u8; 3 << 20]
        );
        assert_eq!(
            std::fs::read(out.join("jobs/1.json")).unwrap(),
            b"{\"a\": 1}"
        );
        // Never into an existing directory.
        assert!(extract(&archive, &out, Limits::default(), Monitor::QUIET).is_err());
    }

    #[test]
    fn unsafe_names_are_refused_when_writing() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["../x", "/x", "a//b", "a/./b", "", "manifest.json", "c:\\x"] {
            let entry = Entry {
                name: name.into(),
                source: Source::Bytes(vec![]),
            };
            assert!(
                write(
                    dir.path(),
                    &mut named("a.zip"),
                    &[entry],
                    "t",
                    serde_json::Value::Null,
                    Monitor::QUIET
                )
                .is_err(),
                "{name}"
            );
        }
        let twice = Entry {
            name: "a".into(),
            source: Source::Bytes(vec![]),
        };
        assert!(
            write(
                dir.path(),
                &mut named("a.zip"),
                &[twice.clone(), twice],
                "t",
                serde_json::Value::Null,
                Monitor::QUIET
            )
            .is_err()
        );
        assert!(!dir.path().join("a.zip").exists());
    }

    #[test]
    fn hostile_archives_are_refused_and_leave_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let refuse = |zip: &Path, limits: Limits, why: &str| {
            let err = extract(zip, &out, limits, Monitor::QUIET).expect_err(why);
            assert!(!out.exists(), "{why}: {err}");
            err.to_string()
        };
        // Zip slip.
        let files: &[(&str, &[u8])] = &[("../evil", b"x")];
        let zip = raw(dir.path(), files, &listing(files));
        assert!(refuse(&zip, Limits::default(), "slip").contains("leaves"));
        assert!(!dir.path().join("evil").exists());
        // A file the manifest does not list.
        let zip = raw(dir.path(), &[("a", b"x")], &listing(&[]));
        assert!(refuse(&zip, Limits::default(), "unlisted").contains("not in"));
        // Content that does not match its hash.
        let zip = raw(dir.path(), &[("a", b"y")], &listing(&[("a", b"x")]));
        assert!(refuse(&zip, Limits::default(), "hash").contains("damaged"));
        // Larger than listed (a lying header or manifest).
        let zip = raw(dir.path(), &[("a", b"xyz")], &listing(&[("a", b"x")]));
        assert!(refuse(&zip, Limits::default(), "size").contains("larger"));
        // A listed file missing.
        let zip = raw(dir.path(), &[], &listing(&[("a", b"x")]));
        assert!(refuse(&zip, Limits::default(), "missing").contains("not in the archive"));
        // Sizes whose sum overflows: refused, never a panic.
        let mut huge = listing(&[("a", b"x"), ("b", b"y")]);
        huge.files[0].bytes = u64::MAX;
        let zip = raw(dir.path(), &[("a", b"x"), ("b", b"y")], &huge);
        refuse(&zip, Limits::default(), "overflow");
        // Too many bytes, too many entries.
        let files: &[(&str, &[u8])] = &[("a", b"0123456789")];
        let zip = raw(dir.path(), files, &listing(files));
        let small = Limits {
            bytes: 5,
            ..Limits::default()
        };
        assert!(refuse(&zip, small, "bytes").contains("bytes"));
        let few = Limits {
            entries: 1,
            ..Limits::default()
        };
        assert!(refuse(&zip, few, "entries").contains("entries"));
        // No manifest, or not a zip at all.
        let path = dir.path().join("plain.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("a", SimpleFileOptions::default()).unwrap();
        zip.finish().unwrap();
        assert!(refuse(&path, Limits::default(), "no manifest").contains(MANIFEST));
        std::fs::write(&path, b"not a zip").unwrap();
        refuse(&path, Limits::default(), "not a zip");
        // A manifest from a newer engine.
        let mut newer = listing(&[]);
        newer.version = MANIFEST_VERSION + 1;
        let zip = raw(dir.path(), &[], &newer);
        assert!(refuse(&zip, Limits::default(), "newer").contains("newer"));
    }

    #[test]
    fn entries_sharing_a_name_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        // Two entries, then the second renamed to the first in the bytes.
        let files: &[(&str, &[u8])] = &[("aaaa1", b"x"), ("aaaa2", b"y")];
        let zip = raw(dir.path(), files, &listing(&files[..1]));
        let mut bytes = std::fs::read(&zip).unwrap();
        let mut i = 0;
        while let Some(at) = bytes[i..].windows(5).position(|w| w == b"aaaa2") {
            bytes[i + at + 4] = b'1';
            i += at + 5;
        }
        std::fs::write(&zip, &bytes).unwrap();
        let out = dir.path().join("out");
        let err = extract(&zip, &out, Limits::default(), Monitor::QUIET).expect_err("duplicate");
        assert!(err.to_string().contains("share a name"), "{err}");
        assert!(!out.exists());
    }

    #[test]
    fn links_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.add_symlink("a", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        zip.start_file(MANIFEST, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&listing(&[])).unwrap())
            .unwrap();
        zip.finish().unwrap();
        let err = extract(
            &path,
            &dir.path().join("out"),
            Limits::default(),
            Monitor::QUIET,
        )
        .expect_err("link");
        assert!(err.to_string().contains("link"), "{err}");
    }

    #[test]
    fn cancelled_writes_leave_no_archive() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        std::fs::write(&data, b"x").unwrap();
        let archive = dir.path().join("a.zip");
        let err = write(
            dir.path(),
            &mut named("a.zip"),
            &[Entry {
                name: "d".into(),
                source: Source::File(data),
            }],
            "t",
            serde_json::Value::Null,
            Monitor::new(&|| true, &|_| {}),
        )
        .expect_err("cancelled");
        assert!(matches!(err, Error::Cancelled));
        assert!(!archive.exists());
        // Only the source file: no temporary file left.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
