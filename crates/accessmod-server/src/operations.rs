//! Operations: long work that is not an analysis (archives, exports,
//! imports) runs on the io lane, with progress, cancellation and pause,
//! and never holds a request.
//!
//! An operation on a project is a run of kind `manage` of that project
//! ([`queue_in_project`]): it is listed with the analyses, and the datasets
//! it reads are protected while it is queued or running. An operation that
//! makes a project (creation, import, demo) has no project to be recorded
//! in yet: it is [`Pending`] in memory while it runs, then recorded in the
//! project it made.

use std::collections::VecDeque;
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
    /// Identifier, unique while the server runs.
    pub id: Id,
    /// `project-create`, `project-import` or `demo-reset`.
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
}

impl PendingOperation {
    /// Whether `principal` may see it.
    pub fn visible_to(&self, principal: &Principal) -> bool {
        principal.admin || principal.user == self.owner
    }
}

/// Operations making projects, in memory.
#[derive(Debug, Default)]
pub struct Pending {
    next: std::sync::atomic::AtomicI64,
    operations: Mutex<VecDeque<PendingOperation>>,
}

impl Pending {
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

    fn add(&self, mut operation: PendingOperation) -> PendingOperation {
        let mut operations = self.lock();
        operation.id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        operations.push_back(operation.clone());
        // Drop the oldest finished ones beyond what is kept.
        while operations.iter().filter(|o| o.finished.is_some()).count() > KEPT {
            if let Some(at) = operations.iter().position(|o| o.finished.is_some()) {
                operations.remove(at);
            }
        }
        operation
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Queue `operation`, making project `slug` named `name`, on the io lane:
/// `work` makes it; once done the operation (with `params`) is recorded as
/// a run of the new project. Returns the queued operation.
pub fn queue_pending(
    state: &AppState,
    principal: &Principal,
    operation: &str,
    slug: &str,
    name: &str,
    params: Value,
    work: impl FnOnce(&AppState, Monitor<'_>) -> ApiResult<(Project, String)> + Send + 'static,
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
        finish_pending(state, id, Err(Error::invalid(e.clone())));
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
            Ok(finish_pending(state, id, Err(Error::Cancelled)).unwrap_or(operation))
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
) -> Option<PendingOperation> {
    let finished = state.pending().update(id, |o| {
        if o.finished.is_some() {
            return;
        }
        o.finished = Some(now());
        o.progress = None;
        match &outcome {
            Ok(()) => o.status = RunStatus::Done,
            Err(error) => {
                o.status = if matches!(error, Error::Cancelled) {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                };
                o.error = Some(RunError {
                    kind: error.kind().name().to_owned(),
                    issues: error.issues(),
                });
            }
        }
    })?;
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
    work: impl FnOnce(&AppState, Monitor<'_>) -> ApiResult<(Project, String)>,
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
    let outcome = outcome.and_then(|(project, outcome)| {
        project.record_operation(
            &running.operation,
            &params,
            running.started,
            &outcome,
            &Value::Null,
        )?;
        state.publish(Event::Projects);
        Ok(())
    });
    let outcome = outcome.map_err(|e| match e {
        ApiError::Engine(error) => error,
        other => Error::invalid(other.to_string()),
    });
    if let Err(e) = &outcome {
        tracing::warn!("{} of {}: {e}", running.operation, running.project);
    }
    finish_pending(state, id, outcome);
}
