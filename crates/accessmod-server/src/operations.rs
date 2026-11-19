//! Operations: long work that is not an analysis (archives, exports,
//! imports) runs on the io lane, with progress, cancellation and pause,
//! and never holds a request.
//!
//! An operation on a project is a run of kind `manage` of that project
//! ([`queue_in_project`]): it is listed with the analyses, and the datasets
//! it reads are protected while it is queued or running. An operation that
//! makes or removes a project (creation, import, demo, removal) has no
//! project to be recorded in: it is [`Pending`] while it runs, then
//! recorded in the project it made. One that no project records (a
//! removal, a creation that failed) is kept in the journal
//! (`<data>/operations.json`), so Jobs lists it after a restart.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use accessmod_io::analysis::{Monitor, Progress};
use accessmod_io::error::Error;
use accessmod_io::project::{Id, Project, RunError, RunRecord, RunStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::Principal;
use crate::error::{ApiError, ApiResult};
use crate::events::{Event, ProgressEvents};
use crate::runner::{Cancel, Control};
use crate::state::AppState;

/// Finished pending operations kept for clients that look later.
const KEPT: usize = 50;

/// Least time between two progress events of a pending operation.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// File of the journal inside the data directory.
pub const JOURNAL_FILE: &str = "operations.json";

/// The lane key of pending operations: no project holds them yet.
pub const PENDING: &str = ":pending";

/// Queue operation `name` of project `slug` with `params`, reading datasets
/// `inputs`, on the io lane; `work` does it and returns its outcome (one
/// line) and its result. `then` publishes what it changed once it is done.
/// Returns the queued run.
pub fn queue_in_project<T: Serialize>(
    state: &AppState,
    slug: &str,
    name: &str,
    params: &Value,
    inputs: &[Id],
    then: fn(&AppState, &str),
    work: impl FnOnce(&Project, Monitor<'_>) -> accessmod_io::error::Result<(String, T)>
    + Send
    + 'static,
) -> ApiResult<RunRecord> {
    let project = state.open(slug)?;
    let id = project.start_operation(name, params, inputs)?;
    let record = project.run_record(id)?;
    publish_run(state, slug, record.clone());
    // A queued task does not keep the state alive (it owns the lanes).
    let worker = state.downgrade();
    let owned = slug.to_owned();
    let queued = state.runner().submit_io(slug, id, move |control| {
        if let Some(state) = worker.upgrade() {
            execute_in_project(&state, &owned, id, control, then, work);
        }
    });
    if let Err(e) = queued {
        let error = Error::invalid(e.clone());
        if project.claim_operation(id)?.is_some() {
            publish_run(state, slug, project.finish_operation(id, Err(&error))?);
        }
        return Err(ApiError::Internal(e));
    }
    Ok(record)
}

fn publish_run(state: &AppState, slug: &str, run: RunRecord) {
    state.publish(Event::Run {
        project: slug.to_owned(),
        run: Box::new(run),
    });
}

fn execute_in_project<T: Serialize>(
    state: &AppState,
    slug: &str,
    id: Id,
    control: &Control,
    then: fn(&AppState, &str),
    work: impl FnOnce(&Project, Monitor<'_>) -> accessmod_io::error::Result<(String, T)>,
) {
    let project = match state.open(slug) {
        Ok(project) => project,
        Err(e) => {
            tracing::error!("operation {id} of {slug}: {e}");
            return;
        }
    };
    // Cancelled while queued: already recorded.
    let running = match project.claim_operation(id) {
        Ok(Some(running)) => running,
        Ok(None) => return,
        Err(e) => {
            tracing::error!("operation {id} of {slug}: {e}");
            return;
        }
    };
    publish_run(state, slug, running.clone());
    let progress = ProgressEvents::new(state.events(), slug, running);
    let report = |p: &Progress| progress.report(p);
    let cancelled = || control.cancelled();
    let monitor = Monitor::new(&cancelled, &report);
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&project, monitor)))
            .unwrap_or_else(|_| Err(Error::invalid("internal error: the operation panicked")))
            .and_then(|(outcome, result)| {
                serde_json::to_value(result)
                    .map(|result| (outcome, result))
                    .map_err(|e| Error::invalid(format!("operation result: {e}")))
            });
    if let Err(e) = &outcome {
        tracing::warn!("operation {id} of {slug}: {e}");
    }
    match project.finish_operation(id, outcome.as_ref().map(Clone::clone)) {
        Ok(record) => publish_run(state, slug, record),
        Err(e) => tracing::error!("operation {id} of {slug}: {e}"),
    }
    if outcome.is_ok() {
        then(state, slug);
    }
}

/// An operation making a project, while it is not recorded in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PendingOperation {
    /// Identifier, unique across restarts (milliseconds since the Unix
    /// epoch when it was queued, or the next one).
    pub id: Id,
    /// `project-create`, `project-import`, `demo-reset` or `project-remove`.
    pub operation: String,
    /// Slug of the project it makes.
    pub project: String,
    /// Name of the project it makes.
    pub name: String,
    /// User who started it.
    pub owner: String,
    /// Queued, running, done, failed or cancelled.
    pub status: RunStatus,
    /// Latest progress while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunError>,
    /// Start, seconds since the Unix epoch.
    pub started: i64,
    /// End, seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<i64>,
    /// Done and recorded as a run of the project it made: listed there.
    #[serde(default)]
    pub recorded: bool,
}

impl PendingOperation {
    /// Whether `principal` may see it.
    pub fn visible_to(&self, principal: &Principal) -> bool {
        principal.admin || principal.user == self.owner
    }
}

/// Operations making or removing projects; those no project records are
/// kept in the journal.
#[derive(Debug, Default)]
pub struct Pending {
    next: std::sync::atomic::AtomicI64,
    operations: Mutex<VecDeque<PendingOperation>>,
    /// The journal file; none: in memory only.
    journal: Option<PathBuf>,
}

impl Pending {
    /// The operations of the journal of `data_dir`: none when the file is
    /// missing or unreadable (with a warning), never an error.
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join(JOURNAL_FILE);
        let kept: Vec<PendingOperation> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("{}: {e}; an empty journal", path.display());
                Vec::new()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                tracing::warn!("{}: {e}; an empty journal", path.display());
                Vec::new()
            }
        };
        // One the server stopped during is interrupted: never resumed.
        let kept: VecDeque<PendingOperation> = kept
            .into_iter()
            .map(|mut o| {
                if o.finished.is_none() {
                    o.status = RunStatus::Interrupted;
                    o.finished = Some(now());
                    o.progress = None;
                }
                o
            })
            .collect();
        Self {
            next: std::sync::atomic::AtomicI64::new(kept.iter().map(|o| o.id).max().unwrap_or(0)),
            operations: Mutex::new(kept),
            journal: Some(path),
        }
    }

    /// Write the operations no project records, unfinished ones too
    /// (atomically: a temporary file renamed over it). A failure is a
    /// warning: the operation goes on.
    fn save(&self, operations: &VecDeque<PendingOperation>) {
        let Some(path) = &self.journal else {
            return;
        };
        let kept: Vec<&PendingOperation> = operations.iter().filter(|o| !o.recorded).collect();
        let temporary = path.with_extension(format!("json.{}", std::process::id()));
        let written = serde_json::to_vec_pretty(&kept)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&temporary, bytes))
            .and_then(|()| std::fs::rename(&temporary, path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temporary);
            tracing::warn!("cannot write {}: {e}", path.display());
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<PendingOperation>> {
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Every operation, oldest first.
    pub fn list(&self) -> Vec<PendingOperation> {
        self.lock().iter().cloned().collect()
    }

    /// Whether an unfinished operation makes project `slug`.
    pub fn reserves(&self, slug: &str) -> bool {
        self.lock()
            .iter()
            .any(|o| o.project == slug && o.finished.is_none())
    }

    fn update(
        &self,
        id: Id,
        change: impl FnOnce(&mut PendingOperation),
    ) -> Option<PendingOperation> {
        let mut operations = self.lock();
        let operation = operations.iter_mut().find(|o| o.id == id)?;
        change(operation);
        Some(operation.clone())
    }

    /// End operation `id` with `outcome` (`recorded`: in the project it
    /// made), unless already ended; the journal is written.
    fn finish(
        &self,
        id: Id,
        outcome: &Result<(), Error>,
        recorded: bool,
    ) -> Option<PendingOperation> {
        let mut operations = self.lock();
        let operation = operations.iter_mut().find(|o| o.id == id)?;
        if operation.finished.is_none() {
            operation.finished = Some(now());
            operation.progress = None;
            match outcome {
                Ok(()) => {
                    operation.status = RunStatus::Done;
                    operation.recorded = recorded;
                }
                Err(error) => {
                    operation.status = if matches!(error, Error::Cancelled) {
                        RunStatus::Cancelled
                    } else {
                        RunStatus::Failed
                    };
                    operation.error = Some(RunError {
                        kind: error.kind().name().to_owned(),
                        issues: error.issues(),
                    });
                }
            }
        }
        let finished = operation.clone();
        self.save(&operations);
        Some(finished)
    }

    fn add(&self, mut operation: PendingOperation) -> PendingOperation {
        let mut operations = self.lock();
        // Ids grow across restarts: the clock, or the next after the last
        // (an id a client saw is never given again).
        let ordering = std::sync::atomic::Ordering::Relaxed;
        let last = self.next.load(ordering);
        operation.id = last.checked_add(1).unwrap_or(last).max(now_ms());
        self.next.store(operation.id, ordering);
        operations.push_back(operation.clone());
        // Drop the oldest finished ones beyond what is kept.
        while operations.iter().filter(|o| o.finished.is_some()).count() > KEPT {
            if let Some(at) = operations.iter().position(|o| o.finished.is_some()) {
                operations.remove(at);
            }
        }
        self.save(&operations);
        operation
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Queue `operation` on project `slug` named `name`, on the io lane:
/// `work` does it; once done, the operation (with `params`) is recorded
/// as a run of the project `work` returns (one it made), or, with none (a
/// removal), kept in the journal. Returns the queued operation.
pub fn queue_pending(
    state: &AppState,
    principal: &Principal,
    operation: &str,
    slug: &str,
    name: &str,
    params: Value,
    work: impl FnOnce(&AppState, Monitor<'_>) -> ApiResult<Option<(Project, String)>> + Send + 'static,
) -> ApiResult<PendingOperation> {
    let queued = state.pending().add(PendingOperation {
        id: 0,
        operation: operation.to_owned(),
        project: slug.to_owned(),
        name: name.to_owned(),
        owner: principal.user.clone(),
        status: RunStatus::Queued,
        progress: None,
        error: None,
        started: now(),
        finished: None,
        recorded: false,
    });
    state.publish(Event::Pending {
        operation: Box::new(queued.clone()),
    });
    let id = queued.id;
    let worker = state.downgrade();
    let submitted = state.runner().submit_io(PENDING, id, move |control| {
        if let Some(state) = worker.upgrade() {
            execute_pending(&state, id, params, control, work);
        }
    });
    if let Err(e) = submitted {
        finish_pending(state, id, Err(Error::invalid(e.clone())), false);
        return Err(ApiError::Internal(e));
    }
    Ok(queued)
}

/// Cancel pending operation `id`: a queued one ends now, a running one at
/// its next check.
pub fn cancel_pending(
    state: &AppState,
    principal: &Principal,
    id: Id,
) -> ApiResult<PendingOperation> {
    let operation = state
        .pending()
        .list()
        .into_iter()
        .find(|o| o.id == id && o.visible_to(principal))
        .ok_or_else(|| ApiError::NotFound(format!("no operation {id}")))?;
    match state.runner().cancel(PENDING, id) {
        // It left its lane before starting: it ends now.
        Cancel::Waiting => {
            Ok(finish_pending(state, id, Err(Error::Cancelled), false).unwrap_or(operation))
        }
        // It ends at its next check, as cancelled or, past its last check, done.
        Cancel::Running => Ok(operation),
        Cancel::None => Err(ApiError::BadRequest(format!(
            "operation {id} is {}: nothing to cancel",
            operation.status.name()
        ))),
    }
}

fn finish_pending(
    state: &AppState,
    id: Id,
    outcome: Result<(), Error>,
    recorded: bool,
) -> Option<PendingOperation> {
    let finished = state.pending().finish(id, &outcome, recorded)?;
    state.publish(Event::Pending {
        operation: Box::new(finished.clone()),
    });
    Some(finished)
}

fn execute_pending(
    state: &AppState,
    id: Id,
    params: Value,
    control: &Control,
    work: impl FnOnce(&AppState, Monitor<'_>) -> ApiResult<Option<(Project, String)>>,
) {
    // Cancelled while queued: already finished.
    let Some(running) = state.pending().update(id, |o| {
        if o.finished.is_none() {
            o.status = RunStatus::Running;
            o.started = now();
        }
    }) else {
        return;
    };
    if running.finished.is_some() {
        return;
    }
    state.publish(Event::Pending {
        operation: Box::new(running.clone()),
    });
    let last: Mutex<Option<(String, Instant)>> = Mutex::new(None);
    let report = |p: &Progress| {
        let now = Instant::now();
        {
            let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
            match &*last {
                Some((stage, at))
                    if *stage == p.stage && now.duration_since(*at) < PROGRESS_INTERVAL =>
                {
                    return;
                }
                _ => *last = Some((p.stage.clone(), now)),
            }
        }
        if let Some(operation) = state.pending().update(id, |o| o.progress = Some(p.clone())) {
            state.publish(Event::Pending {
                operation: Box::new(operation),
            });
        }
    };
    let cancelled = || control.cancelled();
    let monitor = Monitor::new(&cancelled, &report);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(state, monitor)))
        .unwrap_or_else(|_| {
            Err(ApiError::Internal(
                "internal error: the operation panicked".into(),
            ))
        });
    let outcome = outcome.and_then(|made| {
        let recorded = made.is_some();
        if let Some((project, outcome)) = made {
            project.record_operation(
                &running.operation,
                &params,
                running.started,
                &outcome,
                &Value::Null,
            )?;
        }
        state.publish(Event::Projects);
        Ok(recorded)
    });
    let outcome = outcome.map_err(|e| match e {
        ApiError::Engine(error) => error,
        other => Error::invalid(other.to_string()),
    });
    if let Err(e) = &outcome {
        tracing::warn!("{} of {}: {e}", running.operation, running.project);
    }
    let recorded = matches!(outcome, Ok(true));
    finish_pending(state, id, outcome.map(|_| ()), recorded);
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    fn operation(name: &str) -> PendingOperation {
        PendingOperation {
            id: 0,
            operation: name.to_owned(),
            project: "p".to_owned(),
            name: "P".to_owned(),
            owner: "local".to_owned(),
            status: RunStatus::Queued,
            progress: None,
            error: None,
            started: 1,
            finished: None,
            recorded: false,
        }
    }

    #[test]
    fn operations_no_project_records_survive_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pending = Pending::open(dir.path());
        let removed = pending.add(operation("project-remove"));
        let failed = pending.add(operation("project-create"));
        let made = pending.add(operation("project-create"));
        let running = pending.add(operation("project-import"));
        pending.finish(removed.id, &Ok(()), false);
        pending.finish(failed.id, &Err(Error::invalid("no DEM")), false);
        pending.finish(made.id, &Ok(()), true);
        // A second end changes nothing.
        pending.finish(removed.id, &Err(Error::Cancelled), false);
        let again = Pending::open(dir.path());
        let listed: Vec<(Id, RunStatus)> = again.list().iter().map(|o| (o.id, o.status)).collect();
        // The one made is a run of its project; the one the server stopped
        // during is interrupted.
        assert_eq!(
            listed,
            [
                (removed.id, RunStatus::Done),
                (failed.id, RunStatus::Failed),
                (running.id, RunStatus::Interrupted)
            ]
        );
        // Ids grow, across restarts too: never one a client saw.
        assert!(removed.id < failed.id && failed.id < made.id && made.id < running.id);
        assert!(again.add(operation("project-remove")).id > running.id);
        // A journal holding the largest id does not overflow.
        let mut last = operation("project-remove");
        last.id = Id::MAX;
        last.finished = Some(1);
        std::fs::write(
            dir.path().join(JOURNAL_FILE),
            serde_json::to_vec(&[last]).expect("json"),
        )
        .expect("write");
        assert_eq!(
            Pending::open(dir.path())
                .add(operation("project-remove"))
                .id,
            Id::MAX
        );
        // A corrupt journal is an empty one.
        std::fs::write(dir.path().join(JOURNAL_FILE), b"{not json").expect("write");
        assert!(Pending::open(dir.path()).list().is_empty());
    }
}
