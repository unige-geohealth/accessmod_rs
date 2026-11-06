//! Lanes of work: each lane runs its tasks one at a time on its own worker
//! thread, in order. Analyses run on the compute lane, one at a time since
//! the memory budget is per run; operations (archives, exports, imports)
//! on the io lane, so they never wait behind a long analysis. A run
//! survives the browser that started it; after a restart, runs left queued
//! or running are marked interrupted (`Project::recover`).
//!
//! A task polls its [`Control`]: cancelled, or paused. A paused task holds
//! where it polls (its memory and temporary files kept) until it is
//! resumed or cancelled; a paused queued task keeps its place while the
//! next ones start.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use accessmod_io::analysis::{Monitor, Progress};
use accessmod_io::project::{Id, Project, RunStatus};
use accessmod_io::resources::ExecutionSettings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::events::{Event, ProgressEvents};

/// How often a paused task checks whether it may go on.
const PAUSE_POLL: Duration = Duration::from_millis(200);

/// A lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LaneName {
    /// Analyses.
    Compute,
    /// Operations: archives, exports, imports.
    Io,
}

impl LaneName {
    const ALL: [LaneName; 2] = [LaneName::Compute, LaneName::Io];
}

/// Cancellation and pause of one task.
#[derive(Debug, Default)]
pub struct Control {
    cancel: AtomicBool,
    pause: AtomicBool,
}

impl Control {
    /// Whether the task must stop; while it is paused, this waits until it
    /// is resumed (`false`) or cancelled (`true`).
    pub fn cancelled(&self) -> bool {
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return true;
            }
            if !self.pause.load(Ordering::Relaxed) {
                return false;
            }
            std::thread::sleep(PAUSE_POLL);
        }
    }

    fn paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }
}

/// What cancelling a task did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cancel {
    /// It was waiting: it left its lane and will never start.
    Waiting,
    /// It is running: it stops at its next check.
    Running,
    /// No lane holds it.
    None,
}

/// A task of a lane: a run of a project.
pub type Key = (String, Id);

type Job = Box<dyn FnOnce(&Control) + Send>;

/// A task waiting or running in a lane, as clients see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueueEntry {
    /// Lane.
    pub lane: LaneName,
    /// Project slug.
    pub project: String,
    /// Run.
    pub run: Id,
    /// 0: running; 1: next; …
    pub position: u32,
    /// Held by the user.
    pub paused: bool,
}

#[derive(Default)]
struct Queue {
    running: Option<(Key, Arc<Control>)>,
    waiting: VecDeque<(Key, Arc<Control>, Job)>,
    stopped: bool,
}

struct Shared {
    name: LaneName,
    queue: Mutex<Queue>,
    wake: Condvar,
    changed: Box<dyn Fn(LaneName, Vec<QueueEntry>) + Send + Sync>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn entries(&self, queue: &Queue) -> Vec<QueueEntry> {
        let entry = |key: &Key, control: &Control, position: usize| QueueEntry {
            lane: self.name,
            project: key.0.clone(),
            run: key.1,
            position: u32::try_from(position).unwrap_or(u32::MAX),
            paused: control.paused(),
        };
        let running = queue.running.iter().map(|(k, c)| entry(k, c, 0));
        let waiting = queue
            .waiting
            .iter()
            .enumerate()
            .map(|(i, (k, c, _))| entry(k, c, i + 1));
        running.chain(waiting).collect()
    }

    /// Tell clients what the lane holds now.
    fn publish(&self, queue: &Queue) {
        (self.changed)(self.name, self.entries(queue));
    }
}

/// A worker running tasks one at a time.
struct Lane {
    shared: Arc<Shared>,
}

impl Lane {
    fn start(
        name: LaneName,
        changed: Box<dyn Fn(LaneName, Vec<QueueEntry>) + Send + Sync>,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            name,
            queue: Mutex::default(),
            wake: Condvar::new(),
            changed,
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name(format!("accessmod-{name:?}").to_lowercase())
            .spawn(move || work(&worker))?;
        Ok(Self { shared })
    }

    fn submit(&self, key: Key, job: Job) -> Result<(), String> {
        let mut queue = self.shared.lock();
        if queue.stopped {
            return Err("the run worker has stopped".into());
        }
        queue.waiting.push_back((key, Arc::default(), job));
        self.shared.publish(&queue);
        self.shared.wake.notify_all();
        Ok(())
    }

    /// Cancel `key`: a waiting task leaves the lane, a running one is
    /// asked to stop.
    fn cancel(&self, key: &Key) -> Cancel {
        let mut queue = self.shared.lock();
        if let Some((k, control)) = &queue.running
            && k == key
        {
            control.cancel.store(true, Ordering::Relaxed);
            return Cancel::Running;
        }
        let Some(at) = queue.waiting.iter().position(|(k, ..)| k == key) else {
            return Cancel::None;
        };
        queue.waiting.remove(at);
        self.shared.publish(&queue);
        Cancel::Waiting
    }

    /// Pause or resume `key`; `false` when the lane does not hold it.
    fn pause(&self, key: &Key, paused: bool) -> bool {
        let queue = self.shared.lock();
        let control = match &queue.running {
            Some((k, control)) if k == key => Some(control),
            _ => queue
                .waiting
                .iter()
                .find(|(k, ..)| k == key)
                .map(|(_, control, _)| control),
        };
        let Some(control) = control else {
            return false;
        };
        control.pause.store(paused, Ordering::Relaxed);
        self.shared.publish(&queue);
        self.shared.wake.notify_all();
        true
    }

    fn entries(&self) -> Vec<QueueEntry> {
        let queue = self.shared.lock();
        self.shared.entries(&queue)
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        let mut queue = self.shared.lock();
        queue.stopped = true;
        // A running task, paused or not, stops at its next poll.
        if let Some((_, control)) = &queue.running {
            control.cancel.store(true, Ordering::Relaxed);
        }
        drop(queue);
        self.shared.wake.notify_all();
    }
}

/// The worker: the first waiting task that is not paused, until stopped.
fn work(shared: &Shared) {
    loop {
        let (key, control, job) = {
            let mut queue = shared.lock();
            loop {
                if queue.stopped {
                    return;
                }
                if let Some(at) = queue.waiting.iter().position(|(_, c, _)| !c.paused()) {
                    let Some(task) = queue.waiting.remove(at) else {
                        continue;
                    };
                    queue.running = Some((task.0.clone(), task.1.clone()));
                    shared.publish(&queue);
                    break task;
                }
                queue = shared
                    .wake
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        // A panic is caught by the task itself; this one keeps the lane alive.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&control)));
        drop(key);
        let mut queue = shared.lock();
        queue.running = None;
        shared.publish(&queue);
    }
}

/// Handle on the lanes.
pub struct Runner {
    projects: PathBuf,
    settings: ExecutionSettings,
    events: broadcast::Sender<Event>,
    compute: Lane,
    io: Lane,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner").finish_non_exhaustive()
    }
}

impl Runner {
    /// Start the workers. They stop when the runner is dropped.
    pub fn start(
        projects: PathBuf,
        settings: ExecutionSettings,
        events: broadcast::Sender<Event>,
    ) -> std::io::Result<Self> {
        let lane = |name| {
            let events = events.clone();
            Lane::start(
                name,
                Box::new(move |lane, entries| {
                    let _ = events.send(Event::Queue { lane, entries });
                }),
            )
        };
        Ok(Self {
            compute: lane(LaneName::Compute)?,
            io: lane(LaneName::Io)?,
            projects,
            settings,
            events,
        })
    }

    fn lanes(&self) -> [&Lane; 2] {
        [&self.compute, &self.io]
    }

    /// Queue analysis run `run` of `project` (already recorded as queued);
    /// fails when the worker has stopped.
    pub fn enqueue(&self, project: &str, run: Id) -> Result<(), String> {
        let (projects, settings, events) = (
            self.projects.clone(),
            self.settings.clone(),
            self.events.clone(),
        );
        let slug = project.to_owned();
        self.compute.submit(
            (project.to_owned(), run),
            Box::new(move |control| execute(&projects, &settings, &events, &slug, run, control)),
        )
    }

    /// Queue `job` on the io lane under run `run` of `project`.
    pub fn submit_io(
        &self,
        project: &str,
        run: Id,
        job: impl FnOnce(&Control) + Send + 'static,
    ) -> Result<(), String> {
        self.io.submit((project.to_owned(), run), Box::new(job))
    }

    /// Ask run `run` of `project` to stop (a queued one leaves its lane,
    /// decided under the lane's lock: it never starts afterwards).
    pub fn cancel(&self, project: &str, run: Id) -> Cancel {
        let key = (project.to_owned(), run);
        self.lanes()
            .iter()
            .map(|lane| lane.cancel(&key))
            .find(|c| *c != Cancel::None)
            .unwrap_or(Cancel::None)
    }

    /// Pause or resume run `run` of `project`; `false` when no lane holds it.
    pub fn pause(&self, project: &str, run: Id, paused: bool) -> bool {
        let key = (project.to_owned(), run);
        self.lanes().iter().any(|lane| lane.pause(&key, paused))
    }

    /// Every task waiting or running, lane by lane.
    pub fn queue(&self) -> Vec<QueueEntry> {
        LaneName::ALL
            .iter()
            .zip(self.lanes())
            .flat_map(|(_, lane)| lane.entries())
            .collect()
    }
}

/// Execute analysis run `run` of `project`; every outcome is recorded in
/// the project and published. A panic is caught and recorded as a failure.
fn execute(
    projects: &std::path::Path,
    settings: &ExecutionSettings,
    events: &broadcast::Sender<Event>,
    slug: &str,
    run: Id,
    control: &Control,
) {
    let publish = |project: &Project| {
        if let Ok(record) = project.run_record(run) {
            let _ = events.send(Event::Run {
                project: slug.to_owned(),
                run: Box::new(record),
            });
        }
    };
    let project = match Project::open(&projects.join(slug)) {
        Ok(project) => project,
        Err(e) => {
            tracing::error!("run {run} of {slug}: {e}");
            return;
        }
    };
    // Cancelled while queued: nothing to do (already recorded).
    let queued = match project.run_record(run) {
        Ok(record) if record.status == RunStatus::Queued => record,
        _ => return,
    };
    // The first stage announces the run as running (the claim happens in
    // `execute_queued`, right before it).
    let mut running = queued;
    running.status = RunStatus::Running;
    let progress = ProgressEvents::new(events.clone(), slug, running);
    let report = |p: &Progress| progress.report(p);
    let cancelled = || control.cancelled();
    let monitor = Monitor::new(&cancelled, &report);
    let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        project.execute_queued(run, settings, monitor)
    }));
    match started {
        Ok(Ok(record)) => {
            tracing::info!(
                "run {run} of {slug} done: {}",
                record.outcome.as_deref().unwrap_or("")
            );
            let _ = events.send(Event::Catalogue {
                project: slug.to_owned(),
            });
        }
        Ok(Err(e)) => tracing::warn!("run {run} of {slug}: {e}"),
        Err(_) => {
            tracing::error!("run {run} of {slug} panicked");
            let _ = project.mark_failed(run, "internal error: the run panicked");
        }
    }
    publish(&project);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn lane() -> (Lane, mpsc::Receiver<Vec<QueueEntry>>) {
        let (sender, changes) = mpsc::channel();
        let sender = Mutex::new(sender);
        let lane = Lane::start(
            LaneName::Io,
            Box::new(move |_, entries| {
                let _ = sender
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(entries);
            }),
        )
        .unwrap();
        (lane, changes)
    }

    fn key(run: Id) -> Key {
        ("p".to_owned(), run)
    }

    #[test]
    fn a_paused_task_holds_until_resumed_and_a_paused_queued_one_lets_others_pass() {
        let (lane, _) = lane();
        let (done, finished) = mpsc::channel::<Id>();
        let (started, running) = mpsc::channel::<()>();
        let (go, wait) = mpsc::channel::<()>();
        let d1 = done.clone();
        lane.submit(
            key(1),
            Box::new(move |control| {
                let _ = started.send(());
                let _ = wait.recv_timeout(Duration::from_secs(5));
                // Polls like an analysis: holds here while paused.
                if !control.cancelled() {
                    let _ = d1.send(1);
                }
            }),
        )
        .unwrap();
        running.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(lane.pause(&key(1), true));
        go.send(()).unwrap();
        // Paused: no result while it holds.
        assert!(finished.recv_timeout(Duration::from_millis(400)).is_err());
        assert!(lane.entries().iter().any(|e| e.run == 1 && e.paused));
        assert!(lane.pause(&key(1), false));
        assert_eq!(finished.recv_timeout(Duration::from_secs(5)).unwrap(), 1);

        // A paused queued task keeps its place; the next one runs first.
        let (gate, open) = mpsc::channel::<()>();
        lane.submit(
            key(2),
            Box::new(move |_| {
                let _ = open.recv_timeout(Duration::from_secs(5));
            }),
        )
        .unwrap();
        let d3 = done.clone();
        lane.submit(
            key(3),
            Box::new(move |_| {
                let _ = d3.send(3);
            }),
        )
        .unwrap();
        let d4 = done.clone();
        lane.submit(
            key(4),
            Box::new(move |_| {
                let _ = d4.send(4);
            }),
        )
        .unwrap();
        assert!(lane.pause(&key(3), true));
        gate.send(()).unwrap();
        assert_eq!(finished.recv_timeout(Duration::from_secs(5)).unwrap(), 4);
        // Task 4 has returned; the lane drops it right after.
        let mut entries = lane.entries();
        for _ in 0..100 {
            if entries.len() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            entries = lane.entries();
        }
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].run, entries[0].position), (3, 1));
        assert!(lane.pause(&key(3), false));
        assert_eq!(finished.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
    }

    #[test]
    fn cancelling_a_paused_task_stops_it_and_a_waiting_one_leaves_the_lane() {
        let (lane, _) = lane();
        let (done, finished) = mpsc::channel::<bool>();
        let (started, running) = mpsc::channel::<()>();
        let (go, wait) = mpsc::channel::<()>();
        lane.submit(
            key(1),
            Box::new(move |control| {
                let _ = started.send(());
                let _ = wait.recv_timeout(Duration::from_secs(5));
                let _ = done.send(control.cancelled());
            }),
        )
        .unwrap();
        running.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(lane.pause(&key(1), true));
        go.send(()).unwrap();
        lane.submit(key(2), Box::new(|_| {})).unwrap();
        assert_ne!(lane.cancel(&key(2)), Cancel::None);
        assert!(!lane.entries().iter().any(|e| e.run == 2));
        assert!(finished.recv_timeout(Duration::from_millis(300)).is_err());
        assert_ne!(lane.cancel(&key(1)), Cancel::None);
        assert!(finished.recv_timeout(Duration::from_secs(5)).unwrap());
        assert_eq!(lane.cancel(&key(9)), Cancel::None);
    }

    #[test]
    fn a_panicking_task_does_not_stop_the_lane() {
        let (lane, _) = lane();
        lane.submit(key(1), Box::new(|_| panic!("boom"))).unwrap();
        let (done, finished) = mpsc::channel();
        lane.submit(
            key(2),
            Box::new(move |_| {
                let _ = done.send(2);
            }),
        )
        .unwrap();
        assert_eq!(finished.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    }
}
