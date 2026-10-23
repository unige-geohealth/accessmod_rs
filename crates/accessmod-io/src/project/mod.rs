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

mod db;
mod import;
mod runs;

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use import::{ImportOptions, resampling};
pub use runs::{RunError, RunRecord, RunStatus, StartedRun};

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
    /// Last change, seconds since the Unix epoch.
    pub updated: i64,
}

/// Tags as AccessMod 5 keeps them: words of letters and digits, any other
/// character separates words, duplicates removed, order kept.
pub fn normalize_tags<S: AsRef<str>>(tags: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        for word in tag.as_ref().split(|c: char| !c.is_alphanumeric()) {
            if !word.is_empty() && !out.iter().any(|t| t == word) {
                out.push(word.to_owned());
            }
        }
    }
    out
}

/// Seconds since the Unix epoch.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
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
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::invalid("a project needs a name"));
        }
        let existed = root.exists();
        if existed {
            let mut entries = std::fs::read_dir(root).map_err(|_| {
                Error::invalid(format!("{} exists and is not a directory", root.display()))
            })?;
            if entries.next().is_some() {
                return Err(Error::invalid(format!(
                    "{} is not empty: a project is created in a new or empty directory",
                    root.display()
                )));
            }
        }
        // The DEM must define a grid before anything is written.
        GridSpec::open_dem(dem)?;
        let result = Self::create_in(root, name, description, dem, settings, cancelled);
        if result.is_err() {
            // Everything in `root` was written here.
            if existed {
                if let Ok(entries) = std::fs::read_dir(root) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let _ = if path.is_dir() {
                            std::fs::remove_dir_all(&path)
                        } else {
                            std::fs::remove_file(&path)
                        };
                    }
                }
            } else {
                let _ = std::fs::remove_dir_all(root);
            }
        }
        result
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
        let record = project.import_dem(dem, settings, cancelled)?;
        project.set_meta("dem", &record.id.to_string())?;
        Ok(project)
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

    fn db_error(&self) -> impl FnOnce(rusqlite::Error) -> Error + '_ {
        Error::database(&self.db_path)
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(self.db_error())?;
        Ok(())
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

    /// Clean up after a crash: imports left half-done are removed, runs
    /// left running are marked interrupted. Only one process may call this,
    /// when no import or run of this project is in progress (the server, at
    /// start).
    pub fn recover(&self) -> Result<()> {
        let mut stmt = self
            .db
            .prepare("SELECT id, file FROM datasets WHERE state = 'importing'")
            .map_err(self.db_error())?;
        let stale: Vec<(Id, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(self.db_error())?
            .collect::<rusqlite::Result<_>>()
            .map_err(self.db_error())?;
        drop(stmt);
        for (id, file) in stale {
            self.remove_files(&file)?;
            self.db
                .execute("DELETE FROM datasets WHERE id = ?1", [id])
                .map_err(self.db_error())?;
        }
        self.db
            .execute(
                "UPDATE runs SET status = 'interrupted', finished = ?1 WHERE status = 'running'",
                [now()],
            )
            .map_err(self.db_error())?;
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
        self.db
            .execute("DELETE FROM datasets WHERE id = ?1", [id])
            .map_err(self.db_error())?;
        self.remove_files(&record.file)
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
            .expect("one file, one fingerprint");
        self.db
            .execute(
                "UPDATE datasets SET state = 'ready', file = ?1, sha256 = ?2, bytes = ?3,
                 preview = ?4 WHERE id = ?5",
                params![
                    file,
                    fingerprint.sha256,
                    fingerprint.bytes as i64,
                    json(summary)?,
                    id
                ],
            )
            .map_err(self.db_error())?;
        self.dataset(id)
    }

    /// Drop a reserved dataset after a failure.
    fn abandon(&self, id: Id, file: &str) {
        let _ = self.remove_files(file);
        let _ = self.db.execute("DELETE FROM datasets WHERE id = ?1", [id]);
    }

    // ---- saved jobs ----------------------------------------------------

    /// Save a job document: a new job when `id` is `None`.
    pub fn save_job(
        &self,
        id: Option<Id>,
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
            Some(id) => {
                let changed = self
                    .db
                    .execute(
                        "UPDATE jobs SET analysis = ?1, name = ?2, document = ?3, updated = ?4
                         WHERE id = ?5",
                        params![analysis, name, text, now(), id],
                    )
                    .map_err(self.db_error())?;
                if changed == 0 {
                    return Err(Error::invalid(format!("no saved job {id} in this project")));
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
            .prepare("SELECT id, analysis, name, document, updated FROM jobs ORDER BY id")
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
                "SELECT id, analysis, name, document, updated FROM jobs WHERE id = ?1",
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
    updated: i64,
}

fn job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: r.get(0)?,
        analysis: r.get(1)?,
        name: r.get(2)?,
        document: r.get(3)?,
        updated: r.get(4)?,
    })
}

impl JobRow {
    fn decode(self) -> Result<SavedJob> {
        Ok(SavedJob {
            id: self.id,
            analysis: self.analysis,
            name: self.name,
            document: decode(&self.document, "job document")?,
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
        assert_eq!(
            normalize_tags(&["santé publique"]),
            vec!["santé", "publique"]
        );
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
