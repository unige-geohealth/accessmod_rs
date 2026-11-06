//! Data archives of a project (AM5 Data › Archive): zips of chosen datasets
//! and runs, kept in `archives/` and listed newest first.
//!
//! An archive keeps the project layout (`datasets/3.tif`,
//! `runs/7/travel_time_seconds.tif`) and puts the job of each chosen run at
//! its root, `job_run<id>.json`, with its output directory set to
//! `replay_run<id>`. In the extracted folder,
//! `accessmod <analysis> run --job job_run<id>.json` computes the run again
//! when its inputs are in the archive (the manifest says which runs are
//! `replayable`). Overviews (`.ovr`) are left out: they are rebuilt.

use crate::analysis::Monitor;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::runs::job_paths;
use super::{Id, Project, now};
use crate::archive::{self, Entry, Source};
use crate::error::{Error, Result};

/// Directory of the archives of a project.
pub const ARCHIVES_DIR: &str = "archives";

/// `format` of a data archive's manifest.
pub const DATA_ARCHIVE: &str = "accessmod-data";

/// An archive of the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveInfo {
    /// File name, e.g. `am6_export_2026-11-06_143205.zip`.
    pub name: String,
    /// Size, bytes.
    pub bytes: u64,
    /// Last change, seconds since the Unix epoch.
    pub modified: i64,
}

/// A run as an archive describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArchivedRun {
    /// Run identifier in the project.
    pub id: Id,
    /// Analysis name.
    pub analysis: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Job at the root of the archive.
    pub job: String,
    /// Whether every input of the job is in the archive.
    pub replayable: bool,
}

/// A dataset as an archive describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArchivedDataset {
    /// Identifier in the project.
    pub id: Id,
    /// Data class (AccessMod 5 identifier).
    pub class: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Path in the archive (and in the project).
    pub file: String,
}

/// Prefix of an archive name: letters, digits, `-` and `_`; `am6_export`
/// when empty.
fn prefix(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok("am6_export".to_owned());
    }
    if name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::invalid(format!(
            "archive name {name:?}: up to 64 letters, digits, - or _"
        )));
    }
    Ok(name.to_owned())
}

impl Project {
    fn archives_dir(&self) -> PathBuf {
        self.root.join(ARCHIVES_DIR)
    }

    /// Path of archive `name`, which must be one of [`Project::archives`].
    pub fn archive_path(&self, name: &str) -> Result<PathBuf> {
        let plain = !name.is_empty()
            && name.ends_with(".zip")
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            && !name.starts_with('.');
        let missing = || Error::invalid(format!("no archive {name:?} in this project"));
        if !plain {
            return Err(missing());
        }
        // Never a link out of the project.
        let path = self
            .resolve_existing(&Path::new(ARCHIVES_DIR).join(name))
            .map_err(|_| missing())?;
        if !path.is_file() {
            return Err(missing());
        }
        Ok(path)
    }

    /// Archives of the project, newest first.
    pub fn archives(&self) -> Result<Vec<ArchiveInfo>> {
        let dir = self.archives_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::file(&dir)(e)),
        };
        let mut list = Vec::new();
        for entry in entries {
            let entry = entry.map_err(Error::file(&dir))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.archive_path(&name).is_err() {
                continue;
            }
            let meta = entry.metadata().map_err(Error::file(entry.path()))?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            list.push(ArchiveInfo {
                name,
                bytes: meta.len(),
                modified,
            });
        }
        list.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.name.cmp(&a.name)));
        Ok(list)
    }

    /// Remove archive `name`.
    pub fn remove_archive(&self, name: &str) -> Result<()> {
        let path = self.archive_path(name)?;
        std::fs::remove_file(&path).map_err(Error::file(&path))
    }

    /// Archive `datasets` and the jobs of `runs` as `<name>_<UTC time>.zip`
    /// in `archives/`.
    pub fn create_archive(
        &self,
        name: &str,
        datasets: &[Id],
        runs: &[Id],
        monitor: Monitor<'_>,
    ) -> Result<ArchiveInfo> {
        if datasets.is_empty() && runs.is_empty() {
            return Err(Error::invalid(
                "nothing to archive: select datasets or jobs",
            ));
        }
        let prefix = prefix(name)?;
        // Held while files are read: the project is not removed meanwhile. A
        // dataset removed meanwhile makes the archive fail; none is left.
        let _hold = self.activity()?;
        let mut entries = Vec::new();
        let mut archived = Vec::new();
        let mut ids = datasets.to_vec();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            let record = self.dataset(id)?;
            // Never a link out of the project.
            let path = self.resolve_existing(Path::new(&record.file))?;
            entries.push(Entry {
                name: record.file.clone(),
                source: Source::File(path),
            });
            archived.push(ArchivedDataset {
                id,
                class: record.class.code().to_owned(),
                tags: record.tags,
                file: record.file,
            });
        }
        let mut described = Vec::new();
        let mut runs = runs.to_vec();
        runs.sort_unstable();
        runs.dedup();
        for id in runs {
            let run = self.run_record(id)?;
            let mut job = run.job.clone();
            let Some(object) = job.as_object_mut() else {
                return Err(Error::invalid(format!(
                    "run {id}: its job is not an object"
                )));
            };
            object.insert(
                "output_dir".into(),
                serde_json::Value::from(format!("replay_run{id}")),
            );
            let file = format!("job_run{id}.json");
            let text = serde_json::to_vec_pretty(&job)
                .map_err(|e| Error::invalid(format!("run {id}: {e}")))?;
            entries.push(Entry {
                name: file.clone(),
                source: Source::Bytes(text),
            });
            let report = format!("{}/run.json", run.output_dir);
            if let Ok(path) = self.resolve_existing(Path::new(&report)) {
                entries.push(Entry {
                    name: report,
                    source: Source::File(path),
                });
            }
            // Every path the job reads, even of a dataset removed since.
            let replayable = job_paths(&run.analysis, &run.job.to_string())
                .iter()
                .filter(|p| **p != run.output_dir)
                .all(|p| archived.iter().any(|d| d.file == *p));
            described.push(ArchivedRun {
                id,
                analysis: run.analysis,
                tags: run.tags,
                job: file,
                replayable,
            });
        }
        let mut readme = String::from(
            "AccessMod data archive. manifest.json lists every file with its SHA-256.\n\
             Datasets keep their project paths (datasets/, runs/).\n",
        );
        for run in &described {
            readme.push_str(&format!(
                "\nRun {} ({} [{}]): accessmod {} run --job {}{}\n",
                run.id,
                run.analysis,
                run.tags.join(" "),
                run.analysis,
                run.job,
                if run.replayable {
                    ""
                } else {
                    "  (some inputs are not in this archive)"
                }
            ));
        }
        entries.push(Entry {
            name: "README.txt".into(),
            source: Source::Bytes(readme.into_bytes()),
        });
        let content = serde_json::json!({
            "project": self.info()?.name,
            "datasets": archived,
            "runs": described,
        });
        let dir = self.archives_dir();
        std::fs::create_dir_all(&dir).map_err(Error::file(&dir))?;
        let stamp = archive::utc_stamp(now());
        let mut names = (1..).map(|n| match n {
            1 => format!("{prefix}_{stamp}.zip"),
            n => format!("{prefix}_{stamp}_{n}.zip"),
        });
        let (path, _) = archive::write(&dir, &mut names, &entries, DATA_ARCHIVE, content, monitor)?;
        let bytes = std::fs::metadata(&path).map_err(Error::file(&path))?.len();
        Ok(ArchiveInfo {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            bytes,
            modified: now(),
        })
    }
}
