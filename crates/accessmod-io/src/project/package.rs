//! Project files (`.am6p`, in place of AccessMod 5's `.am5p`): a whole
//! project in one zip, to move it to another machine or keep it.
//!
//! The zip holds `project.sqlite` (a consistent snapshot, `VACUUM INTO`,
//! even while the project is used), the file of every ready dataset with its
//! overviews (`.ovr`: a project opens ready to preview), and the `run.json`
//! of each run directory; `manifest.json` lists every file with its SHA-256
//! ([`crate::archive`]). Without results, run outputs are left out and the
//! runs keep only their jobs: a smaller file that replays.
//!
//! Importing treats the file as hostile: it is extracted by
//! [`crate::archive::extract`] into a new directory, its database must pass
//! SQLite's `quick_check`, hold no trigger nor view, open with this engine
//! (migrations, newer versions refused), and name only files inside it.

use crate::analysis::Monitor;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{DATASETS_DIR, DB_FILE, Project, RUNS_DIR, now};
use crate::archive::{self, Entry, Limits, Source};
use crate::error::{Error, Result};

/// `format` of a project file's manifest.
pub const PACKAGE_FORMAT: &str = "am6p";

/// Extension of project files.
pub const PACKAGE_EXTENSION: &str = "am6p";

/// Progress stage of an export: the database snapshot.
pub const STAGE_SNAPSHOT: &str = "snapshot";
/// Progress stage of an import: checking the database and files.
pub const STAGE_CHECK: &str = "check";

/// What a project file holds, in its manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PackageContent {
    /// Project name.
    pub name: String,
    /// Free description.
    pub description: String,
    /// Datasets in the file.
    pub datasets: usize,
    /// Runs recorded.
    pub runs: usize,
    /// Whether run outputs are in the file.
    pub results: bool,
}

/// File name part of `name`: letters, digits, `-` and `_`.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let stem = stem.trim_matches('_').to_owned();
    if stem.is_empty() {
        "project".to_owned()
    } else {
        stem
    }
}

impl Project {
    /// Directory of the project's own exports, a disposable cache: exports
    /// older than an hour are removed by the next one
    /// ([`Project::export_latest`]).
    pub fn exports_dir(&self) -> PathBuf {
        self.cache_dir().join(PACKAGE_EXTENSION)
    }

    /// Export the project to [`Project::exports_dir`], removing exports older
    /// than an hour (time to download them) once it is written; the file name.
    pub fn export_latest(&self, results: bool, monitor: Monitor<'_>) -> Result<String> {
        let dir = self.exports_dir();
        let path = self.export_package(&dir, results, monitor)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                // Earlier exports only, not the temporary files of one in
                // progress, nor one another export just returned.
                let other = entry.file_name().to_string_lossy().into_owned();
                let old = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > std::time::Duration::from_secs(3600));
                if other != name && old && other.ends_with(&format!(".{PACKAGE_EXTENSION}")) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(name)
    }

    /// Path of export `name` of [`Project::exports_dir`].
    pub fn exported_package(&self, name: &str) -> Result<PathBuf> {
        let missing = || Error::invalid(format!("no project file {name:?}"));
        let plain = name.ends_with(&format!(".{PACKAGE_EXTENSION}"))
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
        if !plain {
            return Err(missing());
        }
        let relative = Path::new(super::CACHE_DIR)
            .join(PACKAGE_EXTENSION)
            .join(name);
        let path = self.resolve_existing(&relative).map_err(|_| missing())?;
        if !path.is_file() {
            return Err(missing());
        }
        Ok(path)
    }

    /// Write the project to a new `.am6p` file in `dir` and return its path;
    /// `results`: with run outputs.
    pub fn export_package(
        &self,
        dir: &Path,
        results: bool,
        monitor: Monitor<'_>,
    ) -> Result<PathBuf> {
        // Held while files are read: the project is not removed meanwhile.
        let _hold = self.activity()?;
        let info = self.info()?;
        std::fs::create_dir_all(dir).map_err(Error::file(dir))?;
        let snapshot = dir.join(format!(
            ".snapshot-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let result = self.write_package(
            dir,
            &snapshot,
            &info.name,
            &info.description,
            results,
            monitor,
        );
        let _ = std::fs::remove_file(&snapshot);
        result
    }

    fn write_package(
        &self,
        dir: &Path,
        snapshot: &Path,
        name: &str,
        description: &str,
        results: bool,
        monitor: Monitor<'_>,
    ) -> Result<PathBuf> {
        monitor.stage(STAGE_SNAPSHOT);
        self.db
            .execute("VACUUM INTO ?1", [snapshot.to_string_lossy()])
            .map_err(self.db_error())?;
        let copy = rusqlite::Connection::open(snapshot).map_err(Error::database(snapshot))?;
        // Only ready datasets travel; without results, only those that are
        // not run outputs.
        copy.execute("DELETE FROM datasets WHERE state != 'ready'", [])
            .map_err(Error::database(snapshot))?;
        // Unfinished runs stay behind: this export itself, and analyses whose
        // outputs are partial (imported, they would only read "interrupted").
        copy.execute_batch(
            "DELETE FROM runs WHERE status IN ('queued', 'running');
             DELETE FROM run_inputs WHERE run NOT IN (SELECT id FROM runs);",
        )
        .map_err(Error::database(snapshot))?;
        if !results {
            copy.execute(
                "DELETE FROM datasets WHERE file NOT LIKE ?1",
                [format!("{DATASETS_DIR}/%")],
            )
            .map_err(Error::database(snapshot))?;
        }
        let files: Vec<String> = {
            let mut stmt = copy
                .prepare("SELECT file FROM datasets ORDER BY id")
                .map_err(Error::database(snapshot))?;
            stmt.query_map([], |r| r.get(0))
                .map_err(Error::database(snapshot))?
                .collect::<rusqlite::Result<_>>()
                .map_err(Error::database(snapshot))?
        };
        let run_dirs: Vec<String> = {
            let mut stmt = copy
                .prepare("SELECT output_dir FROM runs WHERE output_dir != '' ORDER BY id")
                .map_err(Error::database(snapshot))?;
            stmt.query_map([], |r| r.get(0))
                .map_err(Error::database(snapshot))?
                .collect::<rusqlite::Result<_>>()
                .map_err(Error::database(snapshot))?
        };
        drop(copy);
        let mut entries = vec![Entry {
            name: DB_FILE.to_owned(),
            source: Source::File(snapshot.to_path_buf()),
        }];
        let mut named = std::collections::HashSet::new();
        let mut add = |entries: &mut Vec<Entry>, name: String| -> Result<()> {
            if named.insert(name.clone()) {
                let path = self.resolve_existing(Path::new(&name))?;
                entries.push(Entry {
                    name,
                    source: Source::File(path),
                });
            }
            Ok(())
        };
        for file in &files {
            add(&mut entries, file.clone())?;
            let overview = format!("{file}.ovr");
            if self.root.join(&overview).is_file() {
                add(&mut entries, overview)?;
            }
        }
        if results {
            // Every file of a run directory: outputs kept only with the run
            // (e.g. nearest_facility.json) and its report too.
            for run_dir in &run_dirs {
                let Ok(dir) = self.resolve_existing(Path::new(run_dir)) else {
                    continue;
                };
                let mut names: Vec<String> = std::fs::read_dir(&dir)
                    .map_err(Error::file(&dir))?
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                for name in names {
                    add(&mut entries, format!("{run_dir}/{name}"))?;
                }
            }
        }
        let content = PackageContent {
            name: name.to_owned(),
            description: description.to_owned(),
            datasets: files.len(),
            runs: run_dirs.len(),
            results,
        };
        let content =
            serde_json::to_value(&content).map_err(|e| Error::invalid(format!("manifest: {e}")))?;
        let stem = file_stem(name);
        let stamp = archive::utc_stamp(now());
        let mut names = (1..).map(|n| match n {
            1 => format!("{stem}_{stamp}.{PACKAGE_EXTENSION}"),
            n => format!("{stem}_{stamp}_{n}.{PACKAGE_EXTENSION}"),
        });
        archive::write(dir, &mut names, &entries, PACKAGE_FORMAT, content, monitor)
            .map(|(path, _)| path)
    }

    /// Import the project file `package` as the new project directory
    /// `target`, which must not exist: extracted beside it, checked, then
    /// moved in place. The project is not the demo, whatever it was.
    pub fn import_package(
        package: &Path,
        target: &Path,
        limits: Limits,
        monitor: Monitor<'_>,
    ) -> Result<Project> {
        if target.exists() {
            return Err(Error::invalid(format!(
                "{} already exists",
                target.display()
            )));
        }
        let parent = target
            .parent()
            .ok_or_else(|| Error::invalid("a project directory needs a parent"))?;
        let staging = parent.join(format!(
            ".import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let checked = archive::extract(package, &staging, limits, monitor).and_then(|manifest| {
            if manifest.format != PACKAGE_FORMAT {
                return Err(Error::invalid(format!(
                    "{} is not an AccessMod project file (format {:?})",
                    package.display(),
                    manifest.format
                )));
            }
            monitor.stage(STAGE_CHECK);
            check_database(&staging.join(DB_FILE))?;
            let project = Project::open(&staging)?;
            project.check_files()?;
            // A usable project: its metadata, grid and DEM.
            let info = project.info()?;
            project.dataset(info.dem)?;
            project
                .db
                .execute("DELETE FROM meta WHERE key = 'demo'", [])
                .map_err(project.db_error())?;
            // Runs left unfinished in the file are interrupted here.
            project
                .db
                .execute(
                    "UPDATE runs SET status = 'interrupted', finished = ?1
                         WHERE status IN ('queued', 'running')",
                    [now()],
                )
                .map_err(project.db_error())?;
            Ok(())
        });
        if let Err(error) = checked {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        if let Err(e) = std::fs::rename(&staging, target) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(Error::file(target)(e));
        }
        Project::open(target)
    }

    /// Every ready dataset's file is inside the project and reads nothing
    /// but itself (as an upload: [`super::check_self_contained`]); no other
    /// file GDAL would read beside a dataset (sidecars) is there.
    fn check_files(&self) -> Result<()> {
        let files: Vec<String> = {
            let mut stmt = self
                .db
                .prepare("SELECT file FROM datasets WHERE state = 'ready'")
                .map_err(self.db_error())?;
            stmt.query_map([], |r| r.get(0))
                .map_err(self.db_error())?
                .collect::<rusqlite::Result<_>>()
                .map_err(self.db_error())?
        };
        for file in files {
            if !(file.starts_with(&format!("{DATASETS_DIR}/"))
                || file.starts_with(&format!("{RUNS_DIR}/")))
            {
                return Err(Error::invalid(format!(
                    "dataset file {file} is not a project path"
                )));
            }
            let path = self.resolve_existing(Path::new(&file))?;
            if !path.is_file() {
                return Err(Error::invalid(format!("dataset file {file} is missing")));
            }
            super::check_self_contained(&path)?;
            let overview = crate::preview::overview_path(&path);
            if overview.is_file() {
                super::check_self_contained(&overview)?;
            }
        }
        let mut pending = vec![self.root.clone()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir)
                .map_err(Error::file(&dir))?
                .flatten()
            {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    pending.push(entry.path());
                } else if SIDECARS.iter().any(|s| name.ends_with(s)) {
                    return Err(Error::invalid(format!(
                        "{name}: a file GDAL would read beside a dataset; project files hold none"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Files GDAL reads beside a raster or a vector on its own (metadata, masks,
/// world files, overviews other than `.ovr` of a dataset, virtual formats).
const SIDECARS: &[&str] = &[
    ".aux.xml", ".aux", ".msk", ".vrt", ".tfw", ".tifw", ".wld", ".rrd", ".ige", ".xml", ".ovr.ovr",
];

/// A database from a file: exactly this engine's schema (checked first,
/// reading nothing but it), then intact.
fn check_database(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(Error::invalid(format!("the project file has no {DB_FILE}")));
    }
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(Error::database(path))?;
    db.pragma_update(None, "trusted_schema", "OFF")
        .map_err(Error::database(path))?;
    super::db::check_schema(&db, path)?;
    let check: String = db
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(Error::database(path))?;
    if check != "ok" {
        return Err(Error::invalid(format!("{DB_FILE} is damaged: {check}")));
    }
    Ok(())
}
