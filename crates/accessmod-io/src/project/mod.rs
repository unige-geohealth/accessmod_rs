//! Projects: datasets on one grid (the DEM's), saved jobs and runs.
//!
//! A project is a directory. Metadata lives in SQLite (`project.sqlite`);
//! data lives in files:
//!
//! ```text
//! <project>/
//!   project.sqlite        project, datasets, saved jobs, runs
//!   datasets/<id>.tif     rasters on the project grid (+ .ovr overviews)
//!   datasets/<id>.fgb     vectors in the project CRS
//!   datasets/<id>.csv     tables
//!   runs/<id>/            outputs of run <id>, job.resolved.json, run.json
//!   archives/<name>.zip   data archives (Data › Archive)
//! ```
//!
//! Jobs of a project name datasets by paths relative to the project
//! (`datasets/3.tif`, `runs/7/travel_time_seconds.tif`) and never leave it,
//! so a job exported from a project replays with the CLI. Run outputs enter
//! the catalogue as datasets of the class their analysis declares
//! ([`crate::analysis::Analysis::OUTPUTS`]).
//!
//! Several processes may use one project (server, CLI): SQLite serialises
//! writes; long work (imports, runs) happens outside transactions, on rows
//! reserved beforehand.

mod archives;
mod db;
mod import;
mod package;
mod runs;

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use archives::{ARCHIVES_DIR, ArchiveInfo, ArchivedDataset, ArchivedRun, DATA_ARCHIVE};
pub use import::{ImportOptions, check_self_contained, resampling};
pub use package::{PACKAGE_EXTENSION, PACKAGE_FORMAT, PackageContent};
pub use runs::{RunError, RunKind, RunRecord, RunStatus};

use crate::analysis::Cancelled;
use crate::class::{DataClass, Kind};
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::preview::Preview;
use crate::resources::ExecutionSettings;

/// Database file of a project.
pub const DB_FILE: &str = "project.sqlite";
/// Directory of datasets.
pub const DATASETS_DIR: &str = "datasets";
/// Directory of runs.
pub const RUNS_DIR: &str = "runs";
/// Directory of disposable caches (preview tiles).
pub const CACHE_DIR: &str = "cache";
/// Lock file: imports and runs hold it shared, recovery exclusive.
pub const LOCK_FILE: &str = ".lock";
/// Suffix of files being deleted.
const DELETING: &str = ".deleting";
/// Mark of a project directory renamed by [`Project::remove`].
const REMOVED: &str = ".removed-";

/// A new name next to `root` for a project being removed.
fn removed_path(root: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut name = root.as_os_str().to_owned();
    name.push(format!("{REMOVED}{}-{nanos:x}", std::process::id()));
    PathBuf::from(name)
}

/// Delete what removals and imports of project files in `projects` (a
/// directory of projects) left; at startup, when no import is in progress.
pub fn purge_removed(projects: &Path) {
    let Ok(entries) = std::fs::read_dir(projects) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(REMOVED) || name.starts_with(".import-") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// A hold on a project while an import or a run works in it; released when
/// dropped.
#[derive(Debug)]
pub struct Activity(#[allow(dead_code)] std::fs::File);

/// Identifier of a dataset, saved job or run (never reused).
pub type Id = i64;

/// A project.
#[derive(Debug)]
pub struct Project {
    root: PathBuf,
    db_path: PathBuf,
    db: Connection,
}

/// Name, grid and DEM of a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectInfo {
    /// Name.
    pub name: String,
    /// Free description.
    pub description: String,
    /// Creation time, seconds since the Unix epoch.
    pub created: i64,
    /// Dataset holding the DEM, which defines the grid.
    pub dem: Id,
    /// The project grid.
    pub grid: GridInfo,
    /// Whether this is the demo project, built from the AccessMod 5 sample
    /// data.
    #[serde(default)]
    pub demo: bool,
}

/// The grid every raster of a project lies on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridInfo {
    /// Columns.
    pub cols: usize,
    /// Rows.
    pub rows: usize,
    /// GDAL geotransform.
    pub geo_transform: [f64; 6],
    /// CRS as WKT.
    pub crs_wkt: String,
}

/// A dataset of the catalogue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetRecord {
    /// Identifier.
    pub id: Id,
    /// Data class.
    pub class: DataClass,
    /// Tags (AccessMod 5): with the class, they name the dataset uniquely.
    pub tags: Vec<String>,
    /// File, relative to the project.
    pub file: String,
    /// Where it comes from.
    pub origin: Origin,
    /// SHA-256 of the file.
    pub sha256: String,
    /// Size of the file, bytes.
    pub bytes: u64,
    /// What it holds, for lists and legends.
    pub summary: Summary,
    /// Creation time, seconds since the Unix epoch.
    pub created: i64,
}

/// Where a dataset comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Origin {
    /// Imported from a file.
    Import {
        /// Source file as given.
        source: String,
        /// Layer read (vectors).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer: Option<String>,
    },
    /// Written by a run.
    Run {
        /// The run.
        run: Id,
        /// File in the run's output directory.
        file: String,
    },
}

/// What a dataset holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Summary {
    /// Raster: overviews and statistics.
    Raster(Preview),
    /// Vector layer.
    Vector(VectorSummary),
    /// Table.
    Table(TableSummary),
}

/// Content of a vector dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VectorSummary {
    /// Features.
    pub features: u64,
    /// OGR geometry type of the layer, e.g. `Point`.
    pub geometry: String,
    /// Attribute fields, in order.
    pub fields: Vec<String>,
}

/// Content of a table dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TableSummary {
    /// Data rows (header excluded).
    pub rows: u64,
    /// Columns of the header.
    pub columns: Vec<String>,
}

/// A saved job: a draft job document, possibly incomplete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SavedJob {
    /// Identifier.
    pub id: Id,
    /// Analysis name (`travel-time`).
    pub analysis: String,
    /// Free name.
    pub name: String,
    /// The job document, paths relative to the project.
    pub document: serde_json::Value,
    /// Revision, increased by every save.
    pub revision: i64,
    /// Last change, seconds since the Unix epoch.
    pub updated: i64,
}

/// Tags as AccessMod 5 keeps them (`amGetUniqueTags`): Latin letters
/// transliterated to ASCII (`santé` → `sante`, `ß` → `ss`), any character
/// but letters and digits separates words, duplicates removed, order kept.
pub fn normalize_tags<S: AsRef<str>>(tags: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let ascii = latin_to_ascii(tag.as_ref());
        for word in ascii.split(|c: char| !c.is_alphanumeric()) {
            if !word.is_empty() && !out.iter().any(|t| t == word) {
                out.push(word.to_owned());
            }
        }
    }
    out
}

/// Latin letters without their diacritics, ligatures spelled out, as ICU's
/// `Latin-ASCII` transform does for the letters tags use. Other scripts
/// are kept.
pub(crate) fn latin_to_ascii(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    use unicode_normalization::char::is_combining_mark;
    let mut out = String::with_capacity(text.len());
    for c in text.nfd().filter(|c| !is_combining_mark(*c)) {
        match c {
            'ß' => out.push_str("ss"),
            'ẞ' => out.push_str("SS"),
            'æ' => out.push_str("ae"),
            'Æ' => out.push_str("AE"),
            'œ' => out.push_str("oe"),
            'Œ' => out.push_str("OE"),
            'ø' => out.push('o'),
            'Ø' => out.push('O'),
            'đ' | 'ð' => out.push('d'),
            'Đ' | 'Ð' => out.push('D'),
            'ł' => out.push('l'),
            'Ł' => out.push('L'),
            'þ' => out.push_str("th"),
            'Þ' => out.push_str("TH"),
            'ı' => out.push('i'),
            other => out.push(other),
        }
    }
    out
}

/// Seconds since the Unix epoch.
/// Metadata key of the project instance ([`Project::instance`]).
const INSTANCE: &str = "instance";

/// A new instance: time, process and a counter.
pub(crate) fn fresh_instance() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{nanos:x}-{:x}-{n:x}", std::process::id())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `root` must not exist, or be an empty directory.
fn check_empty(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(root)
        .map_err(|_| Error::invalid(format!("{} exists and is not a directory", root.display())))?;
    if entries.next().is_some() {
        return Err(Error::invalid(format!(
            "{} is not empty: a project is created in a new or empty directory",
            root.display()
        )));
    }
    Ok(())
}

/// Rename `from` to `to`; nothing to do when `from` does not exist.
fn rename_if_exists(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::file(from)(e)),
    }
}

/// `path` relative to the project: no root, no `..`, not empty.
fn check_relative(path: &Path) -> std::result::Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(format!(
                    "{} leaves the project (paths are relative to the project, without `..`)",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

impl Project {
    /// Create a project in `root` (new or empty directory) on the grid of
    /// `dem`, which becomes its first dataset.
    pub fn create(
        root: &Path,
        name: &str,
        description: &str,
        dem: &Path,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<Self> {
        Self::create_with(
            root,
            name,
            description,
            dem,
            settings,
            cancelled,
            |_| Ok(()),
        )
    }

    /// [`Project::create`], then `populate` (imports, metadata) before the
    /// project appears in `root`: it appears whole or not at all.
    pub fn create_with(
        root: &Path,
        name: &str,
        description: &str,
        dem: &Path,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
        populate: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<Self> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::invalid("a project needs a name"));
        }
        check_empty(root)?;
        // The DEM must define a grid before anything is written.
        GridSpec::open_dem(dem)?;
        // Build the project in a directory of its own next to `root`, then
        // move it in place: a failure removes only what this call wrote.
        let parent = match root.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        std::fs::create_dir_all(&parent).map_err(Error::file(&parent))?;
        let staging = parent.join(format!(
            ".accessmod-project-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir(&staging).map_err(Error::file(&staging))?;
        let built = Self::create_in(&staging, name, description, dem, settings, cancelled)
            .and_then(|project| populate(&project).map(|()| project))
            .and_then(|project| {
                // Close the database before moving its directory.
                drop(project);
                if root.exists() {
                    // Still empty (checked above): only an empty directory
                    // can be removed.
                    std::fs::remove_dir(root).map_err(|_| {
                        Error::invalid(format!(
                            "{} is not empty: a project is created in a new or empty directory",
                            root.display()
                        ))
                    })?;
                }
                std::fs::rename(&staging, root).map_err(Error::file(root))
            });
        if let Err(error) = built {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        Self::open(root)
    }

    fn create_in(
        root: &Path,
        name: &str,
        description: &str,
        dem: &Path,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<Self> {
        for dir in [DATASETS_DIR, RUNS_DIR] {
            let path = root.join(dir);
            std::fs::create_dir_all(&path).map_err(Error::file(&path))?;
        }
        let db_path = root.join(DB_FILE);
        let db = db::open(&db_path, true)?;
        let project = Self {
            root: root.to_path_buf(),
            db_path,
            db,
        };
        project.set_meta("name", name)?;
        project.set_meta("description", description)?;
        project.set_meta("created", &now().to_string())?;
        project.set_meta(INSTANCE, &fresh_instance())?;
        let record = project.import_dem(dem, settings, cancelled)?;
        project.set_meta("dem", &record.id.to_string())?;
        Ok(project)
    }

    /// Hold the project for an import, a run or queuing a run: several may
    /// hold it at once, across processes; [`Project::recover`] and
    /// [`Project::remove`] wait for none. Fails once the project was
    /// removed, even when the removal ended while this call waited.
    pub fn activity(&self) -> Result<Activity> {
        let path = self.root.join(LOCK_FILE);
        if !self.db_path.is_file() {
            return Err(Error::invalid("the project was removed"));
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(Error::file(&path))?;
        file.lock_shared().map_err(Error::file(&path))?;
        // A removal may have ended while this call waited for the lock.
        if !self.db_path.is_file() {
            return Err(Error::invalid("the project was removed"));
        }
        Ok(Activity(file))
    }

    /// Open the project in `root`.
    pub fn open(root: &Path) -> Result<Self> {
        let db_path = root.join(DB_FILE);
        if !db_path.is_file() {
            return Err(Error::invalid(format!(
                "{} is not a project (no {DB_FILE})",
                root.display()
            )));
        }
        let db = db::open(&db_path, false)?;
        Ok(Self {
            root: root.to_path_buf(),
            db_path,
            db,
        })
    }

    /// Project directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of disposable caches; anything in it may be deleted.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(CACHE_DIR)
    }

    fn db_error(&self) -> impl FnOnce(rusqlite::Error) -> Error + '_ {
        Error::database(&self.db_path)
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(self.db_error())?;
        Ok(())
    }

    /// This copy of the project: fresh at creation and at a restore (a
    /// project file keeps its creation time), so a project made again at
    /// the same path is another. Set now for a project made before.
    pub fn instance(&self) -> Result<String> {
        match self.meta(INSTANCE) {
            Ok(instance) => Ok(instance),
            Err(_) => {
                let instance = fresh_instance();
                self.set_meta(INSTANCE, &instance)?;
                Ok(instance)
            }
        }
    }

    fn meta(&self, key: &str) -> Result<String> {
        self.db
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(self.db_error())?
            .ok_or_else(|| Error::invalid(format!("project metadata has no {key:?}")))
    }

    /// Name, grid and DEM.
    pub fn info(&self) -> Result<ProjectInfo> {
        let dem: Id = self
            .meta("dem")?
            .parse()
            .map_err(|_| Error::invalid("project metadata: invalid dem"))?;
        let grid = self.grid()?;
        Ok(ProjectInfo {
            name: self.meta("name")?,
            description: self.meta("description")?,
            created: self.meta("created")?.parse().unwrap_or(0),
            dem,
            demo: self.meta("demo").is_ok_and(|v| v == "1"),
            grid: GridInfo {
                cols: grid.cols,
                rows: grid.rows,
                geo_transform: grid.geo_transform,
                crs_wkt: grid.srs.to_wkt().unwrap_or_default(),
            },
        })
    }

    /// Change the name and description.
    pub fn rename(&self, name: &str, description: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::invalid("a project needs a name"));
        }
        self.set_meta("name", name)?;
        self.set_meta("description", description)
    }

    /// The grid of the project: the DEM's.
    pub fn grid(&self) -> Result<GridSpec> {
        let dem: Id = self
            .meta("dem")?
            .parse()
            .map_err(|_| Error::invalid("project metadata: invalid dem"))?;
        let record = self.dataset(dem)?;
        Ok(GridSpec::open_dem(&self.root.join(&record.file))?.1)
    }

    /// Absolute path of a path relative to the project, checked to stay
    /// inside it.
    pub fn resolve(&self, relative: &Path) -> Result<PathBuf> {
        check_relative(relative).map_err(Error::invalid)?;
        Ok(self.root.join(relative))
    }

    /// [`Project::resolve`] of an existing file, checked to stay inside the
    /// project once symbolic links are resolved: what is read for a client
    /// (archives, downloads) never leaves the project.
    pub fn resolve_existing(&self, relative: &Path) -> Result<PathBuf> {
        let path = self.resolve(relative)?;
        let root = std::fs::canonicalize(&self.root).map_err(Error::file(&self.root))?;
        let real = std::fs::canonicalize(&path).map_err(Error::file(&path))?;
        if !real.starts_with(&root) {
            return Err(Error::invalid(format!(
                "{} leaves the project through a link",
                relative.display()
            )));
        }
        Ok(path)
    }

    /// Clean up after a crash: half-done imports and deletions are removed,
    /// runs left queued or running are marked interrupted. Only when no
    /// process is importing or running in this project (none holds it,
    /// [`Project::activity`]); otherwise nothing is done and `false` is
    /// returned.
    pub fn recover(&self) -> Result<bool> {
        let path = self.root.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(Error::file(&path))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
            Err(std::fs::TryLockError::Error(e)) => return Err(Error::file(&path)(e)),
        }
        let mut stmt = self
            .db
            .prepare(
                "SELECT id, file, state FROM datasets WHERE state IN ('importing', 'deleting')",
            )
            .map_err(self.db_error())?;
        let stale: Vec<(Id, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(self.db_error())?
            .collect::<rusqlite::Result<_>>()
            .map_err(self.db_error())?;
        drop(stmt);
        for (id, file, state) in stale {
            // A run output being registered keeps its file (it belongs to
            // the run); a plain import or a deletion removes it.
            if state == "deleting" || file.starts_with(DATASETS_DIR) {
                self.remove_files(&file)?;
                self.remove_files(&format!("{file}{DELETING}"))?;
            }
            self.db
                .execute("DELETE FROM datasets WHERE id = ?1", [id])
                .map_err(self.db_error())?;
        }
        self.db
            .execute(
                "UPDATE runs SET status = 'interrupted', finished = ?1
                 WHERE status IN ('queued', 'running')",
                [now()],
            )
            .map_err(self.db_error())?;
        // Archives and exports whose writing was interrupted: their
        // temporary files (`.partial`, a database snapshot) start with a dot.
        for dir in [self.root.join(ARCHIVES_DIR), self.exports_dir()] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().starts_with('.') {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
        // Dropping `file` releases the lock.
        Ok(true)
    }

    /// Remove the project and every file in it. Refused while a process
    /// imports or runs in it ([`Project::activity`]) or a run is queued.
    /// The directory is first renamed aside, to a new
    /// `<root>.removed-<id>` that is not a project: from then on the
    /// removal is done. Its files are deleted next; what a failure leaves
    /// is deleted by [`purge_removed`].
    pub fn remove(self) -> Result<()> {
        self.remove_instance(None)
    }

    /// [`Project::remove`], only if it is still `instance` (checked under
    /// the lock that removal holds): a project made again at its path since
    /// is kept.
    pub fn remove_if(self, instance: &str) -> Result<()> {
        self.remove_instance(Some(instance))
    }

    fn remove_instance(self, expected: Option<&str>) -> Result<()> {
        let path = self.root.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(Error::file(&path))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(Error::invalid(
                    "the project is in use by an import or a run; try again when it ends",
                ));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(Error::file(&path)(e)),
        }
        // Read again under the lock, from the directory there now: the
        // connection opened before may be on a project removed and made
        // again at this path meanwhile (nobody can while the lock is held).
        if let Some(expected) = expected
            && Self::open(&self.root)?.instance()? != expected
        {
            return Err(Error::invalid(
                "the project was made again since its removal was asked: nothing removed",
            ));
        }
        let pending: i64 = self
            .db
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE status IN ('queued', 'running')",
                [],
                |r| r.get(0),
            )
            .map_err(self.db_error())?;
        if pending > 0 {
            return Err(Error::invalid(
                "the project has queued or running runs; cancel them first",
            ));
        }
        let Self { root, db, .. } = self;
        drop(db);
        let aside = removed_path(&root);
        if aside.exists() {
            return Err(Error::invalid(format!("{} exists", aside.display())));
        }
        std::fs::rename(&root, &aside).map_err(Error::file(&root))?;
        drop(file);
        // The project is gone; a failure here leaves files for `purge_removed`.
        let _ = std::fs::remove_dir_all(&aside);
        Ok(())
    }

    // ---- datasets ------------------------------------------------------

    /// Datasets, of `class` when given, by identifier.
    pub fn datasets(&self, class: Option<DataClass>) -> Result<Vec<DatasetRecord>> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT id, class, tags, file, origin, sha256, bytes, preview, created
                 FROM datasets WHERE state = 'ready' AND (?1 IS NULL OR class = ?1)
                 ORDER BY id",
            )
            .map_err(self.db_error())?;
        let rows = stmt
            .query_map([class.map(DataClass::code)], dataset_row)
            .map_err(self.db_error())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(self.db_error())?;
        rows.into_iter().map(|row| row.decode()).collect()
    }

    /// Dataset `id`.
    pub fn dataset(&self, id: Id) -> Result<DatasetRecord> {
        self.db
            .query_row(
                "SELECT id, class, tags, file, origin, sha256, bytes, preview, created
                 FROM datasets WHERE state = 'ready' AND id = ?1",
                [id],
                dataset_row,
            )
            .optional()
            .map_err(self.db_error())?
            .ok_or_else(|| Error::invalid(format!("no dataset {id} in this project")))?
            .decode()
    }

    /// Replace the tags of dataset `id`.
    pub fn retag(&self, id: Id, tags: &[String]) -> Result<DatasetRecord> {
        let record = self.dataset(id)?;
        let tags = self.free_tags(record.class, tags, Some(id))?;
        self.db
            .execute(
                "UPDATE datasets SET tags = ?1, tags_key = ?2 WHERE id = ?3",
                params![json(&tags)?, tags.join("_"), id],
            )
            .map_err(self.db_error())?;
        self.dataset(id)
    }

    /// Remove dataset `id` and its files. The DEM cannot be removed.
    pub fn remove_dataset(&self, id: Id) -> Result<()> {
        if self.meta("dem")? == id.to_string() {
            return Err(Error::invalid(
                "the DEM defines the project grid and cannot be removed",
            ));
        }
        let record = self.dataset(id)?;
        if let Some(reader) = self.active_reader(id, None)? {
            return Err(Error::invalid(format!(
                "dataset {id} is read by run {reader}: remove it once that run ends"
            )));
        }
        // Claim the row first: two removals cannot both proceed, and an
        // interrupted one is finished by `recover`. Atomic with the reader
        // check: a run recording this dataset as an input (`link_inputs`)
        // either comes first and keeps it, or finds it being deleted.
        let claimed = self
            .db
            .execute(
                "UPDATE datasets SET state = 'deleting' WHERE id = ?1 AND state = 'ready'
                 AND NOT EXISTS (SELECT 1 FROM run_inputs i JOIN runs r ON r.id = i.run
                     WHERE i.dataset = ?1 AND r.status IN ('queued', 'running'))",
                [id],
            )
            .map_err(self.db_error())?;
        if claimed != 1 {
            if let Some(reader) = self.active_reader(id, None)? {
                return Err(Error::invalid(format!(
                    "dataset {id} is read by run {reader}: remove it once that run ends"
                )));
            }
            return Err(Error::invalid(format!("no dataset {id} in this project")));
        }
        // Move the files aside; on failure the dataset stays as it was.
        let path = self.resolve(Path::new(&record.file))?;
        let aside = self.resolve(Path::new(&format!("{}{DELETING}", record.file)))?;
        let moved = rename_if_exists(&path, &aside).and_then(|()| {
            rename_if_exists(
                &crate::preview::overview_path(&path),
                &crate::preview::overview_path(&aside),
            )
        });
        if let Err(error) = moved {
            let _ = rename_if_exists(&aside, &path);
            let _ = self.db.execute(
                "UPDATE datasets SET state = 'ready' WHERE id = ?1 AND state = 'deleting'",
                [id],
            );
            return Err(error);
        }
        self.db
            .execute("DELETE FROM datasets WHERE id = ?1", [id])
            .map_err(self.db_error())?;
        // Left-overs are removed by `recover` if this fails.
        let _ = self.remove_files(&format!("{}{DELETING}", record.file));
        Ok(())
    }

    /// Remove a dataset file and its side files (overviews).
    fn remove_files(&self, file: &str) -> Result<()> {
        if file.is_empty() {
            return Ok(());
        }
        let path = self.resolve(Path::new(file))?;
        for path in [crate::preview::overview_path(&path), path] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::file(&path)(e)),
            }
        }
        Ok(())
    }

    /// Normalised `tags`, checked non-empty and free for `class` (ignoring
    /// dataset `except`).
    fn free_tags(
        &self,
        class: DataClass,
        tags: &[String],
        except: Option<Id>,
    ) -> Result<Vec<String>> {
        let tags = normalize_tags(tags);
        if tags.is_empty() {
            return Err(Error::invalid("a dataset needs at least one tag"));
        }
        let taken: Option<Id> = self
            .db
            .query_row(
                "SELECT id FROM datasets WHERE class = ?1 AND tags_key = ?2 AND id IS NOT ?3",
                params![class.code(), tags.join("_"), except],
                |r| r.get(0),
            )
            .optional()
            .map_err(self.db_error())?;
        if let Some(other) = taken {
            return Err(Error::invalid(format!(
                "dataset {other} is already a {} tagged {}",
                class.label(),
                tags.join(" ")
            )));
        }
        Ok(tags)
    }

    /// The dataset of `class` tagged `tags` that an output of run `run` would
    /// replace: none when the tags are free; refused without `overwrite`,
    /// for the project DEM, for a dataset being imported or removed, and for
    /// one that another queued or running run reads.
    fn replaceable(
        &self,
        class: DataClass,
        tags: &[String],
        overwrite: bool,
        run: Id,
    ) -> Result<Option<DatasetRecord>> {
        if !overwrite {
            return self.free_tags(class, tags, None).map(|_| None);
        }
        let tags = normalize_tags(tags);
        if tags.is_empty() {
            return Err(Error::invalid("a dataset needs at least one tag"));
        }
        let taken: Option<(Id, String)> = self
            .db
            .query_row(
                "SELECT id, state FROM datasets WHERE class = ?1 AND tags_key = ?2",
                params![class.code(), tags.join("_")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(self.db_error())?;
        let Some((id, state)) = taken else {
            return Ok(None);
        };
        if state != "ready" || self.meta("dem")? == id.to_string() {
            return Err(Error::invalid(format!(
                "dataset {id}, a {} tagged {}, cannot be replaced now",
                class.label(),
                tags.join(" ")
            )));
        }
        if let Some(reader) = self.active_reader(id, Some(run))? {
            return Err(Error::invalid(format!(
                "dataset {id}, a {} tagged {}, is read by run {reader}: it is replaced only once \
                 that run ends",
                class.label(),
                tags.join(" ")
            )));
        }
        self.dataset(id).map(Some)
    }

    /// A queued or running run (other than `except`) that reads dataset
    /// `id`: an analysis on its inputs, an archive or export on its files.
    pub(crate) fn active_reader(&self, id: Id, except: Option<Id>) -> Result<Option<Id>> {
        self.db
            .query_row(
                "SELECT r.id FROM run_inputs i JOIN runs r ON r.id = i.run
                 WHERE i.dataset = ?1 AND r.id IS NOT ?2 AND r.status IN ('queued', 'running')
                 ORDER BY r.id LIMIT 1",
                params![id, except],
                |r| r.get(0),
            )
            .optional()
            .map_err(self.db_error())
    }

    /// Reserve a dataset row (state `importing`) and its file name.
    fn reserve(&self, class: DataClass, tags: &[String], origin: &Origin) -> Result<(Id, String)> {
        let tags = self.free_tags(class, tags, None)?;
        self.db
            .execute(
                "INSERT INTO datasets (class, tags, tags_key, file, state, origin, created)
                 VALUES (?1, ?2, ?3, '', 'importing', ?4, ?5)",
                params![
                    class.code(),
                    json(&tags)?,
                    tags.join("_"),
                    json(origin)?,
                    now()
                ],
            )
            .map_err(self.db_error())?;
        let id = self.db.last_insert_rowid();
        let extension = match class.kind() {
            Kind::Raster => "tif",
            Kind::Vector => "fgb",
            Kind::Table => "csv",
            Kind::Config => "json",
        };
        let file = format!("{DATASETS_DIR}/{id}.{extension}");
        self.db
            .execute(
                "UPDATE datasets SET file = ?1 WHERE id = ?2",
                params![file, id],
            )
            .map_err(self.db_error())?;
        Ok((id, file))
    }

    /// Mark a reserved dataset ready, with its file `file` (relative).
    fn complete(
        &self,
        id: Id,
        file: &str,
        summary: &Summary,
        cancelled: Cancelled<'_>,
    ) -> Result<DatasetRecord> {
        let path = self.resolve(Path::new(file))?;
        let fingerprint = crate::report::fingerprints(&[("dataset", &path)], cancelled)?
            .pop()
            .ok_or_else(|| Error::invalid("no fingerprint"))?;
        let changed = self
            .db
            .execute(
                "UPDATE datasets SET state = 'ready', file = ?1, sha256 = ?2, bytes = ?3,
                 preview = ?4 WHERE id = ?5 AND state = 'importing'",
                params![
                    file,
                    fingerprint.sha256,
                    fingerprint.bytes as i64,
                    json(summary)?,
                    id
                ],
            )
            .map_err(self.db_error())?;
        if changed != 1 {
            return Err(Error::invalid(format!(
                "dataset {id} was removed while it was being imported"
            )));
        }
        self.dataset(id)
    }

    /// Drop a reserved dataset after a failure.
    fn abandon(&self, id: Id, file: &str) {
        let _ = self.remove_files(file);
        let _ = self.db.execute("DELETE FROM datasets WHERE id = ?1", [id]);
    }

    // ---- saved jobs ----------------------------------------------------

    /// Save a job document: a new job when `id` is `None`; otherwise job
    /// `id`, which must still be at `revision` (the one the caller read), so
    /// that two users editing one job cannot overwrite each other silently.
    pub fn save_job(
        &self,
        id: Option<(Id, i64)>,
        analysis: &str,
        name: &str,
        document: &serde_json::Value,
    ) -> Result<SavedJob> {
        if !crate::analysis::job_schemas()
            .iter()
            .any(|(n, _)| *n == analysis)
        {
            return Err(Error::invalid(format!("no analysis named {analysis:?}")));
        }
        let text = serde_json::to_string(document).map_err(|e| Error::invalid(e.to_string()))?;
        let id = match id {
            Some((id, revision)) => {
                let changed = self
                    .db
                    .execute(
                        "UPDATE jobs SET analysis = ?1, name = ?2, document = ?3, updated = ?4,
                         revision = revision + 1 WHERE id = ?5 AND revision = ?6",
                        params![analysis, name, text, now(), id, revision],
                    )
                    .map_err(self.db_error())?;
                if changed == 0 {
                    let current = self.job(id)?;
                    return Err(Error::invalid(format!(
                        "saved job {id} changed since revision {revision} (now {}): reload it",
                        current.revision
                    )));
                }
                id
            }
            None => {
                self.db
                    .execute(
                        "INSERT INTO jobs (analysis, name, document, updated) VALUES (?1, ?2, ?3, ?4)",
                        params![analysis, name, text, now()],
                    )
                    .map_err(self.db_error())?;
                self.db.last_insert_rowid()
            }
        };
        self.job(id)
    }

    /// Saved jobs, by identifier.
    pub fn jobs(&self) -> Result<Vec<SavedJob>> {
        let mut stmt = self
            .db
            .prepare("SELECT id, analysis, name, document, revision, updated FROM jobs ORDER BY id")
            .map_err(self.db_error())?;
        let rows = stmt
            .query_map([], job_row)
            .map_err(self.db_error())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(self.db_error())?;
        rows.into_iter().map(|r| r.decode()).collect()
    }

    /// Saved job `id`.
    pub fn job(&self, id: Id) -> Result<SavedJob> {
        self.db
            .query_row(
                "SELECT id, analysis, name, document, revision, updated FROM jobs WHERE id = ?1",
                [id],
                job_row,
            )
            .optional()
            .map_err(self.db_error())?
            .ok_or_else(|| Error::invalid(format!("no saved job {id} in this project")))?
            .decode()
    }

    /// Remove saved job `id`.
    pub fn remove_job(&self, id: Id) -> Result<()> {
        let removed = self
            .db
            .execute("DELETE FROM jobs WHERE id = ?1", [id])
            .map_err(self.db_error())?;
        if removed == 0 {
            return Err(Error::invalid(format!("no saved job {id} in this project")));
        }
        Ok(())
    }
}

/// JSON text of `value` for a database column.
fn json(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::invalid(format!("cannot encode: {e}")))
}

/// JSON column `text` decoded.
fn decode<T: serde::de::DeserializeOwned>(text: &str, what: &str) -> Result<T> {
    serde_json::from_str(text)
        .map_err(|e| Error::invalid(format!("project database: invalid {what} ({e})")))
}

/// Raw dataset row.
struct DatasetRow {
    id: Id,
    class: String,
    tags: String,
    file: String,
    origin: String,
    sha256: Option<String>,
    bytes: Option<i64>,
    preview: Option<String>,
    created: i64,
}

fn dataset_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DatasetRow> {
    Ok(DatasetRow {
        id: r.get(0)?,
        class: r.get(1)?,
        tags: r.get(2)?,
        file: r.get(3)?,
        origin: r.get(4)?,
        sha256: r.get(5)?,
        bytes: r.get(6)?,
        preview: r.get(7)?,
        created: r.get(8)?,
    })
}

impl DatasetRow {
    fn decode(self) -> Result<DatasetRecord> {
        Ok(DatasetRecord {
            id: self.id,
            class: DataClass::from_code(&self.class).ok_or_else(|| {
                Error::invalid(format!("project database: unknown class {}", self.class))
            })?,
            tags: decode(&self.tags, "tags")?,
            file: self.file,
            origin: decode(&self.origin, "origin")?,
            sha256: self.sha256.unwrap_or_default(),
            bytes: self.bytes.unwrap_or(0) as u64,
            summary: decode(self.preview.as_deref().unwrap_or("null"), "summary")?,
            created: self.created,
        })
    }
}

/// Raw saved-job row.
struct JobRow {
    id: Id,
    analysis: String,
    name: String,
    document: String,
    revision: i64,
    updated: i64,
}

fn job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: r.get(0)?,
        analysis: r.get(1)?,
        name: r.get(2)?,
        document: r.get(3)?,
        revision: r.get(4)?,
        updated: r.get(5)?,
    })
}

impl JobRow {
    fn decode(self) -> Result<SavedJob> {
        Ok(SavedJob {
            id: self.id,
            analysis: self.analysis,
            name: self.name,
            document: decode(&self.document, "job document")?,
            revision: self.revision,
            updated: self.updated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_split_on_anything_but_letters_and_digits() {
        assert_eq!(
            normalize_tags(&["Road network; 2024", "road", "network"]),
            vec!["Road", "network", "2024", "road"]
        );
        assert_eq!(normalize_tags(&["  ", "--"]), Vec::<String>::new());
        // Latin-ASCII, as AccessMod 5 (`stri_trans_general`).
        assert_eq!(
            normalize_tags(&["santé publique"]),
            vec!["sante", "publique"]
        );
        assert_eq!(
            normalize_tags(&["Straße", "Ærø", "œuvre", "Łódź"]),
            vec!["Strasse", "AEro", "oeuvre", "Lodz"]
        );
        assert_eq!(normalize_tags(&["_a__b_ "]), vec!["a", "b"]);
    }

    #[test]
    fn relative_paths_stay_inside() {
        assert!(check_relative(Path::new("datasets/3.tif")).is_ok());
        assert!(check_relative(Path::new("./runs/7/x.tif")).is_ok());
        assert!(check_relative(Path::new("../other/x.tif")).is_err());
        assert!(check_relative(Path::new("datasets/../../x")).is_err());
        assert!(check_relative(Path::new("/etc/passwd")).is_err());
        assert!(check_relative(Path::new("")).is_err());
    }
}
