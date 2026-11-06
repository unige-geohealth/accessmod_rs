//! Uploads: files sent by the browser, staged in the data directory until a
//! command (`project.create`, `dataset.import`) takes them.
//!
//! An upload is a directory `<data>/uploads/<id>/` holding one or more files
//! (a shapefile comes with its `.dbf`, `.shx`, `.prj`) and the user who sent
//! them. Commands name it with a [`FileSource`]; a command that succeeds
//! removes it, and uploads left over for a day are purged.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Multipart, State};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::auth::{Principal, Requirement};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Directory of uploads inside the data directory.
pub const UPLOADS_DIR: &str = "uploads";
/// File of an upload naming the user who sent it.
const OWNER_FILE: &str = ".owner";
/// Files in one upload.
const MAX_FILES: usize = 32;
/// Age after which an unused upload is removed.
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// Companion files: never the file a command reads.
const SIDECARS: &[&str] = &[
    "dbf", "shx", "prj", "cpg", "qix", "sbn", "sbx", "xml", "ovr", "aux", "qmd", "ige", "rrd",
    "rde",
];

/// A file a command reads: on the server's disk (desktop) or uploaded.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FileSource {
    /// A path on the server's disk, when the server allows local paths.
    Path(String),
    /// An upload of the same user.
    Upload {
        /// Upload identifier, from `POST /api/uploads`.
        id: String,
        /// The file to read, when the upload holds several main files.
        #[serde(default)]
        file: Option<String>,
    },
}

/// Response of `POST /api/uploads`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Upload {
    /// Identifier, for a [`FileSource::Upload`].
    pub id: String,
    /// Names of the files received.
    pub files: Vec<String>,
}

/// A file resolved from a [`FileSource`], and the upload to remove once the
/// command has succeeded.
#[derive(Debug)]
pub struct Resolved {
    /// File to read.
    pub path: PathBuf,
    /// The upload and its lock: no purge or other command touches it
    /// until this is dropped.
    upload: Option<(PathBuf, std::fs::File)>,
}

impl Resolved {
    /// The command succeeded: the upload is no longer needed.
    pub fn consumed(self) {
        if let Some((dir, _lock)) = self.upload {
            remove(&dir);
        }
    }
}

/// Lock upload `dir` (through its owner file) for its sole use; `None`
/// when another request holds it.
fn lock(dir: &Path) -> std::io::Result<Option<std::fs::File>> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(OWNER_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// `POST /api/uploads`: multipart, one part per file.
pub async fn upload(
    State(state): State<AppState>,
    principal: Principal,
    mut multipart: Multipart,
) -> ApiResult<Json<Upload>> {
    principal.authorize(Requirement::User, None)?;
    let root = state.uploads_dir();
    purge(&root, MAX_AGE);
    let id = new_id();
    let dir = root.join(&id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(format!("cannot create an upload: {e}")))?;
    match receive(
        &mut multipart,
        &dir,
        &principal,
        state.config().upload_limit,
    )
    .await
    {
        Ok(files) => Ok(Json(Upload { id, files })),
        Err(error) => {
            remove(&dir);
            Err(error)
        }
    }
}

async fn receive(
    multipart: &mut Multipart,
    dir: &Path,
    principal: &Principal,
    limit: u64,
) -> ApiResult<Vec<String>> {
    tokio::fs::write(dir.join(OWNER_FILE), principal.user.as_bytes())
        .await
        .map_err(|e| ApiError::Internal(format!("cannot write an upload: {e}")))?;
    // Held while receiving: a purge skips the upload.
    let _lock = lock(dir)
        .ok()
        .flatten()
        .ok_or_else(|| ApiError::Internal("cannot lock a new upload".into()))?;
    let mut files = Vec::new();
    let mut total: u64 = 0;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("invalid upload: {e}")))?
    {
        let name = field
            .file_name()
            .ok_or_else(|| ApiError::BadRequest("every part of an upload must be a file".into()))
            .and_then(file_name)?;
        if files.contains(&name) {
            return Err(ApiError::BadRequest(format!("{name:?} sent twice")));
        }
        if files.len() == MAX_FILES {
            return Err(ApiError::BadRequest(format!(
                "at most {MAX_FILES} files per upload"
            )));
        }
        let path = dir.join(&name);
        let mut out = tokio::fs::File::create_new(&path)
            .await
            .map_err(|e| ApiError::Internal(format!("cannot write {name:?}: {e}")))?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::BadRequest(format!("upload interrupted: {e}")))?
        {
            total = total.saturating_add(chunk.len() as u64);
            if total > limit {
                return Err(ApiError::BadRequest(format!(
                    "the upload exceeds {} MiB",
                    limit >> 20
                )));
            }
            out.write_all(&chunk)
                .await
                .map_err(|e| ApiError::Internal(format!("cannot write {name:?}: {e}")))?;
        }
        out.flush()
            .await
            .map_err(|e| ApiError::Internal(format!("cannot write {name:?}: {e}")))?;
        files.push(name);
    }
    if files.is_empty() {
        return Err(ApiError::BadRequest("the upload holds no file".into()));
    }
    Ok(files)
}

/// A file name sent by a browser, kept only when it is a plain name: no
/// directory, no hidden file, no control character.
pub fn file_name(name: &str) -> ApiResult<String> {
    let invalid = || ApiError::BadRequest(format!("invalid file name {name:?}"));
    let plain = !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control);
    if plain && Path::new(name).file_name() == Some(name.as_ref()) {
        Ok(name.to_owned())
    } else {
        Err(invalid())
    }
}

/// Resolve `source` for `principal`: a file GDAL reads, which must read
/// nothing but itself and its companion files.
pub fn resolve(
    state: &AppState,
    principal: &Principal,
    source: &FileSource,
) -> ApiResult<Resolved> {
    resolve_file(state, principal, source, true)
}

/// Resolve `source` for `principal`: a zip read by
/// [`accessmod_io::archive::extract`] (a project file), never by GDAL.
pub fn resolve_archive(
    state: &AppState,
    principal: &Principal,
    source: &FileSource,
) -> ApiResult<Resolved> {
    resolve_file(state, principal, source, false)
}

fn resolve_file(
    state: &AppState,
    principal: &Principal,
    source: &FileSource,
    gdal: bool,
) -> ApiResult<Resolved> {
    match source {
        FileSource::Path(path) => Ok(Resolved {
            path: state.local_path(path)?,
            upload: None,
        }),
        FileSource::Upload { id, file } => {
            let missing = || ApiError::NotFound(format!("no upload {id:?}"));
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(missing());
            }
            let dir = state.uploads_dir().join(id);
            let mut guard = match lock(&dir) {
                Ok(Some(guard)) => guard,
                Ok(None) => {
                    return Err(ApiError::BadRequest(format!(
                        "upload {id:?} is in use by another request"
                    )));
                }
                Err(_) => return Err(missing()),
            };
            let mut owner = String::new();
            std::io::Read::read_to_string(&mut guard, &mut owner).map_err(|_| missing())?;
            if owner != principal.user {
                return Err(missing());
            }
            let names = upload_files(&dir)?;
            let name = match file {
                Some(file) => {
                    let file = file_name(file)?;
                    if !names.contains(&file) {
                        return Err(ApiError::NotFound(format!(
                            "no file {file:?} in upload {id:?}"
                        )));
                    }
                    file
                }
                None => main_file(&names)?,
            };
            let path = dir.join(name);
            // A file sent by a user must not make the server read others.
            if gdal {
                accessmod_io::project::check_self_contained(&path)?;
            }
            Ok(Resolved {
                path,
                upload: Some((dir, guard)),
            })
        }
    }
}

fn upload_files(dir: &Path) -> ApiResult<Vec<String>> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| ApiError::Internal(format!("cannot read an upload: {e}")))?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n != OWNER_FILE)
        .collect();
    names.sort();
    Ok(names)
}

/// The one file of `names` that is not a companion file.
fn main_file(names: &[String]) -> ApiResult<String> {
    let mains: Vec<&String> = names
        .iter()
        .filter(|n| {
            let ext = Path::new(n.as_str())
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            !SIDECARS.contains(&ext.as_str())
        })
        .collect();
    match mains.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(ApiError::BadRequest(
            "the upload holds only companion files (.dbf, .prj, …)".into(),
        )),
        _ => Err(ApiError::BadRequest(format!(
            "the upload holds several files ({}): name the one to read",
            mains
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}{:x}{n:04x}", std::process::id())
}

fn remove(dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        tracing::warn!("cannot remove upload {}: {e}", dir.display());
    }
}

/// Remove uploads older than `age`.
pub fn purge(root: &Path, age: Duration) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|d| d > age);
        if !old {
            continue;
        }
        // An upload being received or consumed is locked: skip it.
        match lock(&entry.path()) {
            Ok(None) => {}
            Ok(Some(_held)) => remove(&entry.path()),
            Err(_) => remove(&entry.path()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_plain() {
        for bad in [
            "", ".", "..", "../x.tif", "a/b.tif", "a\\b.tif", ".owner", "c:x", "a\nb",
        ] {
            assert!(file_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            file_name("roads 2024.shp").ok().as_deref(),
            Some("roads 2024.shp")
        );
    }

    #[test]
    fn the_main_file_skips_companions() {
        let names = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            main_file(&names(&["r.dbf", "r.prj", "r.shp", "r.shx"]))
                .ok()
                .as_deref(),
            Some("r.shp")
        );
        assert!(main_file(&names(&["a.tif", "b.tif"])).is_err());
        assert!(main_file(&names(&["r.dbf"])).is_err());
    }
}
