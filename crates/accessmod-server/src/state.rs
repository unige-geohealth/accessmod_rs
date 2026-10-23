//! Server state: configuration, projects directory, events and runner.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use accessmod_io::project::Project;
use accessmod_io::resources::ExecutionSettings;
use tokio::sync::broadcast;

use crate::error::{ApiError, ApiResult};
use crate::events::Event;
use crate::runner::Runner;

/// Directory of projects inside the data directory.
pub const PROJECTS_DIR: &str = "projects";

/// Server configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Data directory: projects (and, later, accounts).
    pub data_dir: PathBuf,
    /// Memory, threads and scratch directory of runs and imports.
    pub settings: ExecutionSettings,
    /// Whether requests may name files on the server's disk (desktop: the
    /// user's own files). A country server takes uploads instead.
    pub local_paths: bool,
}

/// Shared state of the server.
#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    config: Config,
    events: broadcast::Sender<Event>,
    runner: Runner,
}

impl AppState {
    /// State over `config`: creates the projects directory, recovers every
    /// project left by a previous process, and starts the runner.
    pub fn new(config: Config) -> ApiResult<Self> {
        let projects = config.data_dir.join(PROJECTS_DIR);
        std::fs::create_dir_all(&projects).map_err(|e| {
            ApiError::Internal(format!("cannot create {}: {e}", projects.display()))
        })?;
        for slug in project_slugs(&projects) {
            match Project::open(&projects.join(&slug)).and_then(|p| p.recover()) {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("project {slug}: in use by another process, not recovered")
                }
                Err(e) => tracing::warn!("project {slug}: recovery failed: {e}"),
            }
        }
        let (events, _) = broadcast::channel(1024);
        let runner = Runner::start(projects, config.settings.clone(), events.clone());
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                events,
                runner,
            }),
        })
    }

    /// Configuration.
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// Directory holding the projects.
    pub fn projects_dir(&self) -> PathBuf {
        self.inner.config.data_dir.join(PROJECTS_DIR)
    }

    /// Directory of project `slug`, which must exist.
    pub fn project_dir(&self, slug: &str) -> ApiResult<PathBuf> {
        check_slug(slug)?;
        let dir = self.projects_dir().join(slug);
        if dir.join(accessmod_io::project::DB_FILE).is_file() {
            Ok(dir)
        } else {
            Err(ApiError::NotFound(format!("no project {slug:?}")))
        }
    }

    /// Open project `slug`.
    pub fn open(&self, slug: &str) -> ApiResult<Project> {
        Ok(Project::open(&self.project_dir(slug)?)?)
    }

    /// Slugs of every project, sorted.
    pub fn project_slugs(&self) -> Vec<String> {
        project_slugs(&self.projects_dir())
    }

    /// Publish an event; nobody listening is fine.
    pub fn publish(&self, event: Event) {
        let _ = self.inner.events.send(event);
    }

    /// Receive the events published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    /// The run queue.
    pub fn runner(&self) -> &Runner {
        &self.inner.runner
    }

    /// A file named by a request, when local paths are allowed.
    pub fn local_path(&self, path: &str) -> ApiResult<PathBuf> {
        if !self.inner.config.local_paths {
            return Err(ApiError::Forbidden(
                "this server takes uploads, not paths on its disk".into(),
            ));
        }
        Ok(PathBuf::from(path))
    }

    /// Run blocking work (SQLite, GDAL) off the async threads; a panic
    /// becomes an internal error instead of taking the server down.
    pub async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(AppState) -> ApiResult<T> + Send + 'static,
    ) -> ApiResult<T> {
        let state = self.clone();
        tokio::task::spawn_blocking(move || work(state))
            .await
            .map_err(|e| ApiError::Internal(format!("task failed: {e}")))?
    }
}

/// Project slugs: lower-case ASCII letters, digits and `-`, 1 to 64.
pub fn check_slug(slug: &str) -> ApiResult<()> {
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(ApiError::NotFound(format!("no project {slug:?}")))
    }
}

/// A free slug for a project named `name`.
pub fn new_slug(projects: &Path, name: &str) -> String {
    let mut base = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('-') && !base.is_empty() {
            base.push('-');
        }
    }
    let base: String = base.trim_end_matches('-').chars().take(48).collect();
    let base = if base.is_empty() {
        "project".to_owned()
    } else {
        base
    };
    let mut slug = base.clone();
    let mut n = 2;
    while projects.join(&slug).exists() {
        slug = format!("{base}-{n}");
        n += 1;
    }
    slug
}

fn project_slugs(projects: &Path) -> Vec<String> {
    let mut slugs: Vec<String> = std::fs::read_dir(projects)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(accessmod_io::project::DB_FILE).is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|s| check_slug(s).is_ok())
        .collect();
    slugs.sort();
    slugs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            new_slug(dir.path(), "Santé Publique — Niger 2024"),
            "sant-publique-niger-2024"
        );
        assert_eq!(new_slug(dir.path(), "  "), "project");
        std::fs::create_dir(dir.path().join("demo")).unwrap();
        assert_eq!(new_slug(dir.path(), "Demo"), "demo-2");
        assert!(check_slug("../etc").is_err());
        assert!(check_slug("demo-2").is_ok());
        assert!(check_slug("").is_err());
    }
}
