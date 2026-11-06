//! Server-sent events: run states and progress, catalogue changes.

use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use accessmod_io::analysis::Progress;
use accessmod_io::project::{Id, RunRecord};
use axum::extract::{Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};

use crate::auth::{Principal, Requirement, Role};
use crate::error::ApiResult;
use crate::runner::{LaneName, QueueEntry};
use crate::state::AppState;

/// An event, sent as JSON with its `type` as the SSE event name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A run was queued, started, finished, failed or was cancelled.
    Run {
        /// Project slug.
        project: String,
        /// The run as it is now.
        run: Box<RunRecord>,
    },
    /// Progress of a running run.
    Progress {
        /// Project slug.
        project: String,
        /// Run identifier.
        run: Id,
        /// Stage and counts.
        progress: Progress,
    },
    /// Runs of a project were removed.
    Runs {
        /// Project slug.
        project: String,
    },
    /// Datasets or saved jobs of a project changed.
    Catalogue {
        /// Project slug.
        project: String,
    },
    /// Projects were created, renamed or deleted.
    Projects,
    /// An operation making a project (creation, import, demo) changed.
    Pending {
        /// The operation as it is now.
        operation: Box<crate::operations::PendingOperation>,
    },
    /// A pre-analysis changed (its client follows it).
    Check {
        /// The pre-analysis as it is now.
        check: Box<crate::checks::Check>,
    },
    /// What a lane holds now: its running task, then the waiting ones.
    Queue {
        /// Lane.
        lane: LaneName,
        /// Tasks, running first.
        entries: Vec<QueueEntry>,
    },
}

impl Event {
    fn name(&self) -> &'static str {
        match self {
            Event::Run { .. } => "run",
            Event::Progress { .. } => "progress",
            Event::Runs { .. } => "runs",
            Event::Catalogue { .. } => "catalogue",
            Event::Projects => "projects",
            Event::Queue { .. } => "queue",
            Event::Pending { .. } => "pending",
            Event::Check { .. } => "check",
        }
    }

    fn project(&self) -> Option<&str> {
        match self {
            Event::Run { project, .. }
            | Event::Progress { project, .. }
            | Event::Runs { project }
            | Event::Catalogue { project } => Some(project),
            Event::Check { check } => Some(&check.project),
            Event::Projects | Event::Queue { .. } | Event::Pending { .. } => None,
        }
    }
}

/// Query of `GET /api/events`.
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Only events of this project (and project list changes).
    pub project: Option<String>,
}

/// `GET /api/events`: the event stream. Besides [`Event`]s, a `resync`
/// event tells a client that fell behind to reload its queries.
pub async fn stream(
    State(state): State<AppState>,
    principal: Principal,
    Query(query): Query<EventsQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    if let Some(project) = &query.project {
        principal.authorize(Requirement::Project(Role::Viewer), Some(project))?;
        state.project_dir(project)?;
    }
    let filter = query.project;
    let events = BroadcastStream::new(state.subscribe()).filter_map(move |event| {
        // A lagging client missed events: tell it to reload what it shows.
        let Ok(mut event) = event else {
            return Some(Ok(SseEvent::default().event("resync").data("{}")));
        };
        // A lane holds tasks of every project: only those this client follows
        // and may see.
        if let Event::Pending { operation } = &event
            && !operation.visible_to(&principal)
        {
            return None;
        }
        if let Event::Queue { entries, .. } = &mut event {
            entries.retain(|e| {
                filter.as_ref().is_none_or(|wanted| *wanted == e.project)
                    && principal
                        .authorize(Requirement::Project(Role::Viewer), Some(&e.project))
                        .is_ok()
            });
        }
        let wanted = match (&filter, event.project()) {
            (Some(wanted), Some(project)) => wanted == project,
            _ => true,
        };
        if !wanted
            || principal
                .authorize(Requirement::Project(Role::Viewer), event.project())
                .is_err()
        {
            return None;
        }
        let data = serde_json::to_string(&event).ok()?;
        Some(Ok(SseEvent::default().event(event.name()).data(data)))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// Least time between two progress events of one stage.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Progress of one run turned into events: each stage when it starts, then
/// counts at most every [`PROGRESS_INTERVAL`].
#[derive(Debug)]
pub struct ProgressEvents {
    events: broadcast::Sender<Event>,
    project: String,
    /// The run, as running: published with the first stage.
    run: RunRecord,
    last: Mutex<Option<(String, Instant)>>,
}

impl ProgressEvents {
    /// Events of run `run` (as running) of `project`.
    pub fn new(events: broadcast::Sender<Event>, project: &str, run: RunRecord) -> Self {
        Self {
            events,
            project: project.to_owned(),
            run,
            last: Mutex::new(None),
        }
    }

    /// Report `progress`; never blocks on clients.
    pub fn report(&self, progress: &Progress) {
        let now = Instant::now();
        {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            match &mut *last {
                Some((stage, at)) if *stage == progress.stage => {
                    if now.duration_since(*at) < PROGRESS_INTERVAL {
                        return;
                    }
                    *at = now;
                }
                None => {
                    *last = Some((progress.stage.clone(), now));
                    let _ = self.events.send(Event::Run {
                        project: self.project.clone(),
                        run: Box::new(self.run.clone()),
                    });
                }
                _ => *last = Some((progress.stage.clone(), now)),
            }
        }
        let _ = self.events.send(Event::Progress {
            project: self.project.clone(),
            run: self.run.id,
            progress: progress.clone(),
        });
    }
}
