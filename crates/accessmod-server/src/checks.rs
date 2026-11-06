//! Pre-analyses: the full validation of a job against its inputs
//! (`prepare`: values on barriers, empty zones, classes without a speed…),
//! run on the check lane while the user edits, with progress, instead of a
//! request that blocks and keeps running once abandoned.
//!
//! The latest pre-analysis of a client (a browser tab), project and
//! analysis wins: a new one cancels the previous one. Results stay in
//! memory a short while; a client follows them through events.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use accessmod_io::analysis::{Monitor, Progress};
use accessmod_io::error::Error;
use accessmod_io::project::{Id, RunError, RunStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::events::Event;
use crate::runner::{Cancel, Control};
use crate::state::AppState;

/// Pre-analyses kept once finished.
const KEPT: usize = 200;

/// Least time between two progress events of a pre-analysis.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Unfinished pre-analyses at most, of every client: a hostile client
/// cannot fill the server.
const ACTIVE_MAX: usize = 32;

/// Longest client token.
const CLIENT_MAX: usize = 64;

/// The lane key of the pre-analyses of project `slug`: apart from its runs.
pub fn lane_key(slug: &str) -> String {
    format!(":check/{slug}")
}

/// A pre-analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    /// Identifier, unique while the server runs.
    pub id: Id,
    /// Project slug.
    pub project: String,
    /// Analysis name.
    pub analysis: String,
    /// Token of the client (browser tab) that asked for it.
    pub client: String,
    /// Queued, running, done, failed or cancelled.
    pub status: RunStatus,
    /// Latest progress while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    /// The validation summary of a done one (`validate --json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Why it failed: the job's issues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunError>,
}

/// Pre-analyses, in memory.
#[derive(Debug, Default)]
pub struct Checks {
    next: std::sync::atomic::AtomicI64,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// The latest pre-analysis of each client, project and analysis, while
    /// it is unfinished.
    latest: HashMap<(String, String, String), Id>,
    checks: VecDeque<Check>,
}

impl Checks {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Pre-analysis `id`.
    pub fn get(&self, id: Id) -> Option<Check> {
        self.lock().checks.iter().find(|c| c.id == id).cloned()
    }

    fn update(&self, id: Id, change: impl FnOnce(&mut Check)) -> Option<Check> {
        let mut inner = self.lock();
        let check = inner.checks.iter_mut().find(|c| c.id == id)?;
        change(check);
        let check = check.clone();
        if !matches!(check.status, RunStatus::Queued | RunStatus::Running) {
            let key = (
                check.client.clone(),
                check.project.clone(),
                check.analysis.clone(),
            );
            if inner.latest.get(&key) == Some(&id) {
                inner.latest.remove(&key);
            }
        }
        Some(check)
    }
}

fn finished(status: RunStatus) -> bool {
    !matches!(status, RunStatus::Queued | RunStatus::Running)
}

/// Queue the pre-analysis of `document`, a job of `analysis` in project
/// `slug`, for `client`; the previous one of that client, project and
/// analysis is cancelled. Returns the queued pre-analysis.
pub fn start(
    state: &AppState,
    slug: &str,
    analysis: &str,
    client: &str,
    document: Value,
) -> ApiResult<Check> {
    if client.is_empty()
        || client.len() > CLIENT_MAX
        || !client
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(ApiError::BadRequest(
            "client: 1 to 64 letters, digits or -".into(),
        ));
    }
    if accessmod_io::analysis::by_name(analysis, NoOp).is_none() {
        return Err(ApiError::NotFound(format!("no analysis {analysis:?}")));
    }
    let checks = state.checks();
    let id = checks
        .next
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        + 1;
    let check = Check {
        id,
        project: slug.to_owned(),
        analysis: analysis.to_owned(),
        client: client.to_owned(),
        status: RunStatus::Queued,
        progress: None,
        result: None,
        error: None,
    };
    let worker = state.downgrade();
    let owned = slug.to_owned();
    // Registered, the previous one cancelled and this one submitted under the
    // registry's lock: a concurrent request for the same client, project
    // and analysis sees this one submitted, never half done.
    let (previous, submitted) = {
        let mut inner = checks.lock();
        let unfinished = inner.checks.iter().filter(|c| !finished(c.status)).count();
        if unfinished >= ACTIVE_MAX {
            return Err(ApiError::BadRequest(
                "the server is busy with pre-analyses: try again in a moment".into(),
            ));
        }
        inner.checks.push_back(check.clone());
        while inner.checks.len() > KEPT {
            match inner.checks.iter().position(|c| finished(c.status)) {
                Some(at) => {
                    inner.checks.remove(at);
                }
                None => break,
            }
        }
        let previous = inner.latest.insert(
            (client.to_owned(), slug.to_owned(), analysis.to_owned()),
            id,
        );
        let previous = previous.map(|p| (p, state.runner().cancel(&lane_key(slug), p)));
        // Announced before the worker may start it: its events arrive in order.
        state.publish(Event::Check {
            check: Box::new(check.clone()),
        });
        let submitted = state
            .runner()
            .submit_check(&lane_key(slug), id, move |control| {
                if let Some(state) = worker.upgrade() {
                    execute(&state, &owned, id, document, control);
                }
            });
        (previous, submitted)
    };
    // A previous one that had not started ends now; a running one at its
    // next check.
    if let Some((previous, Cancel::Waiting)) = previous {
        finish(state, previous, Err(Error::Cancelled));
    }
    if let Err(e) = submitted {
        finish(state, id, Err(Error::invalid(e.clone())));
        return Err(ApiError::Internal(e));
    }
    Ok(check)
}

struct NoOp;

impl accessmod_io::analysis::ForAnalysis for NoOp {
    type Output = ();
    fn call<A: accessmod_io::analysis::Analysis>(self) {}
}

fn finish(state: &AppState, id: Id, outcome: Result<Value, Error>) {
    let done = state.checks().update(id, |c| {
        if finished(c.status) {
            return;
        }
        c.progress = None;
        match outcome {
            Ok(result) => {
                c.status = RunStatus::Done;
                c.result = Some(result);
            }
            Err(error) => {
                c.status = if matches!(error, Error::Cancelled) {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                };
                c.error = Some(RunError {
                    kind: error.kind().name().to_owned(),
                    issues: error.issues(),
                });
            }
        }
    });
    if let Some(check) = done {
        state.publish(Event::Check {
            check: Box::new(check),
        });
    }
}

fn execute(state: &AppState, slug: &str, id: Id, document: Value, control: &Control) {
    let Some(running) = state.checks().update(id, |c| {
        if !finished(c.status) {
            c.status = RunStatus::Running;
        }
    }) else {
        return;
    };
    if finished(running.status) {
        return;
    }
    state.publish(Event::Check {
        check: Box::new(running.clone()),
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
        if let Some(check) = state.checks().update(id, |c| c.progress = Some(p.clone())) {
            state.publish(Event::Check {
                check: Box::new(check),
            });
        }
    };
    let cancelled = || control.cancelled();
    let monitor = Monitor::new(&cancelled, &report);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state
            .open(slug)
            .map_err(|e| match e {
                ApiError::Engine(error) => error,
                other => Error::invalid(other.to_string()),
            })?
            .validate_named(
                &running.analysis,
                &document,
                &state.config().settings,
                monitor,
            )
    }))
    .unwrap_or_else(|_| Err(Error::invalid("internal error: the pre-analysis panicked")));
    finish(state, id, outcome);
}
