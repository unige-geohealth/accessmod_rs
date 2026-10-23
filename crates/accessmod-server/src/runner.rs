//! The run queue: runs execute one at a time on a worker thread, since the
//! memory budget is per run. A run survives the browser that started it;
//! after a restart, runs left queued or running are marked interrupted
//! (`Project::recover`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use accessmod_io::analysis::{Monitor, Progress};
use accessmod_io::project::{Id, Project, RunStatus};
use accessmod_io::resources::ExecutionSettings;
use tokio::sync::broadcast;

use crate::events::{Event, ProgressEvents};

/// A run to execute.
#[derive(Debug)]
struct Task {
    project: String,
    run: Id,
}

type Flags = Arc<Mutex<HashMap<(String, Id), Arc<AtomicBool>>>>;

/// Handle on the run queue.
#[derive(Debug)]
pub struct Runner {
    tasks: mpsc::Sender<Task>,
    /// Cancellation flag of every queued or running run.
    flags: Flags,
}

impl Runner {
    /// Start the worker. It stops when the runner is dropped.
    pub fn start(
        projects: PathBuf,
        settings: ExecutionSettings,
        events: broadcast::Sender<Event>,
    ) -> std::io::Result<Self> {
        let (tasks, queue) = mpsc::channel::<Task>();
        let flags: Flags = Arc::default();
        let worker_flags = flags.clone();
        std::thread::Builder::new()
            .name("accessmod-runner".into())
            .spawn(move || {
                for task in queue {
                    let flag = lock(&worker_flags)
                        .get(&(task.project.clone(), task.run))
                        .cloned()
                        .unwrap_or_default();
                    execute(&projects, &settings, &events, &task, &flag);
                    lock(&worker_flags).remove(&(task.project, task.run));
                }
            })?;
        Ok(Self { tasks, flags })
    }

    /// Queue run `run` of `project` (already recorded as queued); fails when
    /// the worker has stopped.
    pub fn enqueue(&self, project: &str, run: Id) -> Result<(), String> {
        let key = (project.to_owned(), run);
        lock(&self.flags).insert(key.clone(), Arc::default());
        let task = Task {
            project: project.to_owned(),
            run,
        };
        if self.tasks.send(task).is_err() {
            lock(&self.flags).remove(&key);
            return Err("the run worker has stopped".into());
        }
        Ok(())
    }

    /// Ask run `run` of `project` to stop; `false` when it is not queued or
    /// running here.
    pub fn cancel(&self, project: &str, run: Id) -> bool {
        match lock(&self.flags).get(&(project.to_owned(), run)) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Execute one task; every outcome is recorded in the project and
/// published. A panic is caught and recorded as a failure.
fn execute(
    projects: &std::path::Path,
    settings: &ExecutionSettings,
    events: &broadcast::Sender<Event>,
    task: &Task,
    flag: &AtomicBool,
) {
    let publish = |project: &Project| {
        if let Ok(run) = project.run_record(task.run) {
            let _ = events.send(Event::Run {
                project: task.project.clone(),
                run,
            });
        }
    };
    let project = match Project::open(&projects.join(&task.project)) {
        Ok(project) => project,
        Err(e) => {
            tracing::error!("run {} of {}: {e}", task.run, task.project);
            return;
        }
    };
    // Cancelled while queued: nothing to do (already recorded).
    let queued = match project.run_record(task.run) {
        Ok(record) if record.status == RunStatus::Queued => record,
        _ => return,
    };
    // The first stage announces the run as running (the claim happens in
    // `execute_queued`, right before it).
    let mut running = queued;
    running.status = RunStatus::Running;
    let progress = ProgressEvents::new(events.clone(), &task.project, running);
    let report = |p: &Progress| progress.report(p);
    let cancelled = || flag.load(Ordering::Relaxed);
    let monitor = Monitor::new(&cancelled, &report);
    let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        project.execute_queued(task.run, settings, monitor)
    }));
    match started {
        Ok(Ok(record)) => {
            tracing::info!(
                "run {} of {} done: {}",
                task.run,
                task.project,
                record.outcome.as_deref().unwrap_or("")
            );
            let _ = events.send(Event::Catalogue {
                project: task.project.clone(),
            });
        }
        Ok(Err(e)) => tracing::warn!("run {} of {}: {e}", task.run, task.project),
        Err(_) => {
            tracing::error!("run {} of {} panicked", task.run, task.project);
            let _ = project.mark_failed(task.run, "internal error: the run panicked");
        }
    }
    publish(&project);
}
