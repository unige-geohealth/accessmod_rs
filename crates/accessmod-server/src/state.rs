//! Server state: configuration, projects directory, events and runner.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use accessmod_io::project::Project;
use accessmod_io::resources::ExecutionSettings;
use tokio::sync::broadcast;

use crate::error::{ApiError, ApiResult};
use crate::events::Event;
use crate::runner::Runner;
use crate::settings::Settings;

/// Directory of projects inside the data directory.
pub const PROJECTS_DIR: &str = "projects";
/// Slug of the demo project ([`accessmod_io::demo`]), never given to
/// another project.
pub const DEMO_SLUG: &str = "demo";

/// Default of [`Config::upload_limit`]: 4 GiB.
pub const DEFAULT_UPLOAD_LIMIT: u64 = 4 << 30;

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
    /// Bytes one upload may hold ([`DEFAULT_UPLOAD_LIMIT`]).
    pub upload_limit: u64,
    /// Build the demo project when the data directory has no projects
    /// directory yet (a new installation).
    pub demo: bool,
    /// MapTiler key of the online basemaps (a public browser key,
    /// restricted to the client's origins); `None`: offline basemap only.
    pub maptiler_key: Option<String>,
}

/// Shared state of the server.
#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

/// A handle on the state that does not keep it alive: what a queued task
/// holds, so that dropping the state stops the lanes.
#[derive(Debug, Clone)]
pub struct WeakState(std::sync::Weak<Inner>);

impl WeakState {
    /// The state, while it lives.
    pub fn upgrade(&self) -> Option<AppState> {
        self.0.upgrade().map(|inner| AppState { inner })
    }
}

#[derive(Debug)]
struct Inner {
    config: Config,
    events: broadcast::Sender<Event>,
    runner: Runner,
    /// Settings, as last written to the data directory.
    settings: Mutex<Settings>,
    /// Held while the demo project is rebuilt.
    demo: Mutex<()>,
    /// Operations making projects, in memory.
    pending: crate::operations::Pending,
}

impl AppState {
    /// State over `config`: creates the projects directory (with the demo
    /// project in a new data directory), recovers every project left by a
    /// previous process, and starts the runner.
    pub fn new(config: Config) -> ApiResult<Self> {
        let projects = config.data_dir.join(PROJECTS_DIR);
        let new = !projects.exists();
        std::fs::create_dir_all(&projects).map_err(|e| {
            ApiError::Internal(format!("cannot create {}: {e}", projects.display()))
        })?;
        if new && config.demo {
            // A failure leaves a working server without the demo, which
            // the toolbox can build again.
            if let Err(e) =
                accessmod_io::demo::create(&projects.join(DEMO_SLUG), &config.settings, &|| false)
            {
                tracing::warn!("cannot build the demo project: {e}");
            }
        }
        for slug in project_slugs(&projects) {
            match Project::open(&projects.join(&slug)).and_then(|p| p.recover()) {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("project {slug}: in use by another process, not recovered")
                }
                Err(e) => tracing::warn!("project {slug}: recovery failed: {e}"),
            }
        }
        accessmod_io::project::purge_removed(&projects);
        crate::uploads::purge(
            &config.data_dir.join(crate::uploads::UPLOADS_DIR),
            std::time::Duration::from_secs(24 * 3600),
        );
        let (events, _) = broadcast::channel(1024);
        let runner = Runner::start(projects, config.settings.clone(), events.clone())
            .map_err(|e| ApiError::Internal(format!("cannot start the run worker: {e}")))?;
        let settings = Mutex::new(Settings::load(&config.data_dir));
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                events,
                runner,
                settings,
                demo: Mutex::new(()),
                pending: crate::operations::Pending::default(),
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

    /// Directory of uploads.
    pub fn uploads_dir(&self) -> PathBuf {
        self.inner.config.data_dir.join(crate::uploads::UPLOADS_DIR)
    }

    /// Open project `slug`.
    pub fn open(&self, slug: &str) -> ApiResult<Project> {
        Ok(Project::open(&self.project_dir(slug)?)?)
    }

    /// Slugs of every project, sorted.
    pub fn project_slugs(&self) -> Vec<String> {
        project_slugs(&self.projects_dir())
    }

    /// Current settings.
    pub fn settings(&self) -> Settings {
        self.inner
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Change the settings with `f` and write them; unchanged on error.
    /// Settings changes are serialised: what `f` checks (a project exists)
    /// holds until they are written.
    pub fn update_settings(
        &self,
        f: impl FnOnce(&mut Settings) -> ApiResult<()>,
    ) -> ApiResult<Settings> {
        let mut current = self
            .inner
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = current.clone();
        f(&mut next)?;
        if next != *current {
            next.save(&self.inner.config.data_dir)?;
            *current = next.clone();
        }
        Ok(next)
    }

    /// Build the demo project again from the embedded data, then put it in
    /// place of the current one (removed once its imports and runs are
    /// over): a failed build leaves the current demo as it was.
    pub fn reset_demo(&self, cancelled: accessmod_io::analysis::Cancelled<'_>) -> ApiResult<()> {
        let _guard = self
            .inner
            .demo
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let projects = self.projects_dir();
        let root = projects.join(DEMO_SLUG);
        let current = if root.exists() {
            let project = Project::open(&root)?;
            if !project.info()?.demo {
                return Err(ApiError::BadRequest(format!(
                    "project {DEMO_SLUG:?} is not the demo project: remove it first"
                )));
            }
            Some(project)
        } else {
            None
        };
        // Built aside, under a name no project has; a crash leaves it to the
        // purge of removed projects at the next start.
        let built = projects.join(format!(
            ".{DEMO_SLUG}.removed-new-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let made = accessmod_io::demo::create(&built, &self.inner.config.settings, cancelled);
        if let Err(error) = made {
            let _ = std::fs::remove_dir_all(&built);
            return Err(error.into());
        }
        let replaced = current
            .map_or(Ok(()), |p| p.remove().map_err(ApiError::from))
            .and_then(|()| {
                std::fs::rename(&built, &root).map_err(|e| {
                    ApiError::Internal(format!("cannot move the demo project in place: {e}"))
                })
            });
        if replaced.is_err() {
            let _ = std::fs::remove_dir_all(&built);
        }
        replaced
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

    /// A handle that does not keep the state alive.
    pub fn downgrade(&self) -> WeakState {
        WeakState(Arc::downgrade(&self.inner))
    }

    /// Operations making projects.
    pub fn pending(&self) -> &crate::operations::Pending {
        &self.inner.pending
    }

    /// The event channel, for workers publishing progress.
    pub fn events(&self) -> broadcast::Sender<Event> {
        self.inner.events.clone()
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
    new_slug_except(projects, name, &|_| false)
}

/// [`new_slug`], also skipping the slugs `reserved` says are taken.
pub fn new_slug_except(projects: &Path, name: &str, reserved: &dyn Fn(&str) -> bool) -> String {
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
    while slug == DEMO_SLUG || projects.join(&slug).exists() || reserved(&slug) {
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
        // A reserved slug is skipped, however long the name.
        let long = "a".repeat(60);
        let first = new_slug(dir.path(), &long);
        let second = new_slug_except(dir.path(), &long, &|s| s == first);
        assert_ne!(first, second);
        assert!(check_slug(&second).is_ok());
        assert!(check_slug("../etc").is_err());
        assert!(check_slug("demo-2").is_ok());
        assert!(check_slug("").is_err());
    }
}
