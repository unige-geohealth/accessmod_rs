//! Runs of analyses inside a project.
//!
//! A run takes a job document whose paths are relative to the project. The
//! project sets its output directory (`runs/<id>`), checks that no path
//! leaves the project, runs the analysis exactly as the CLI does, and
//! registers the outputs as datasets tagged with the run's tags.

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};

use super::import::summarise;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Id, Origin, Project, RUNS_DIR, check_relative, decode, json, now};
use crate::analysis::{Analysis, ForAnalysis, Monitor, by_name};
use crate::error::{Error, Result};
use crate::job::JobFile;
use crate::resources::ExecutionSettings;

/// State of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Waiting for its turn.
    Queued,
    /// In progress.
    Running,
    /// Finished; outputs registered.
    Done,
    /// Stopped by an error.
    Failed,
    /// Cancelled by the user.
    Cancelled,
    /// Stopped by a crash or restart; outputs are incomplete.
    Interrupted,
}

impl RunStatus {
    /// Name in JSON and in the database (`queued`, `running`, …).
    pub fn name(self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Done => "done",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Interrupted => "interrupted",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Done,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::Interrupted,
        ]
        .into_iter()
        .find(|s| s.name() == code)
    }
}

/// What a run does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// An analysis: a job, inputs, outputs registered as datasets.
    #[default]
    Analysis,
    /// An operation on the project (archive, export, import): its
    /// `analysis` is the operation name, its `job` the parameters.
    Manage,
}

impl RunKind {
    /// Name in JSON and in the database.
    pub fn name(self) -> &'static str {
        match self {
            RunKind::Analysis => "analysis",
            RunKind::Manage => "manage",
        }
    }
}

/// Why a run failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunError {
    /// `validation`, `io`, `computation`, `cancelled` or `internal`.
    pub kind: String,
    /// Every problem found.
    pub issues: Vec<String>,
}

/// A run of the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunRecord {
    /// Identifier.
    pub id: Id,
    /// Analysis name.
    pub analysis: String,
    /// Tags of the datasets it writes.
    pub tags: Vec<String>,
    /// Job document, paths relative to the project.
    pub job: serde_json::Value,
    /// State.
    pub status: RunStatus,
    /// Output directory, relative to the project.
    pub output_dir: String,
    /// One line summarising the result (done runs).
    pub outcome: Option<String>,
    /// Why it failed (failed and cancelled runs).
    pub error: Option<RunError>,
    /// Start, seconds since the Unix epoch.
    pub started: i64,
    /// End, seconds since the Unix epoch.
    pub finished: Option<i64>,
    /// Datasets it wrote.
    pub datasets: Vec<Id>,
    /// Datasets its job reads, still in the project.
    pub inputs: Vec<Id>,
    /// Whether it may replace datasets of its output classes and tags.
    #[serde(default)]
    pub overwrite: bool,
    /// Analysis or operation.
    #[serde(default)]
    pub kind: RunKind,
    /// What a done operation made (an archive, a file, a dataset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

/// `path` as the catalogue stores files: components joined by `/`, without
/// `.` components.
fn catalogue_key(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Catalogue keys of the paths of `job`, a job of `analysis`; none when
/// the analysis is unknown or the job no longer parses.
pub(super) fn job_paths(analysis: &str, job: &str) -> Vec<String> {
    struct Paths<'a>(&'a str);
    impl ForAnalysis for Paths<'_> {
        type Output = Vec<String>;
        fn call<A: Analysis>(self) -> Vec<String> {
            let Ok(mut job) = A::Job::from_json(self.0) else {
                return Vec::new();
            };
            let mut paths = Vec::new();
            job.for_each_path(&mut |path| paths.push(catalogue_key(path)));
            paths
        }
    }
    by_name(analysis, Paths(job)).unwrap_or_default()
}

impl Project {
    /// Job document for analysis `A` with `output_dir` set, typed and
    /// checked: every input path names a ready dataset of this project
    /// whose class its field accepts, and stays inside the project once
    /// symbolic links are resolved. Paths come back absolute, with the
    /// datasets they name.
    fn project_job<A: Analysis>(
        &self,
        document: &serde_json::Value,
        output_dir: &str,
    ) -> Result<(serde_json::Value, A::Job, Vec<Id>)> {
        let mut document = document.clone();
        let Some(object) = document.as_object_mut() else {
            return Err(Error::invalid("a job document is a JSON object"));
        };
        object.insert("output_dir".into(), serde_json::Value::from(output_dir));
        let mut job = A::Job::from_json(&document.to_string())?;
        let mut paths = Vec::new();
        let mut issues = Vec::new();
        job.for_each_path(&mut |path: &mut PathBuf| match check_relative(path) {
            Ok(()) => paths.push(catalogue_key(path)),
            Err(issue) => issues.push(issue),
        });
        if !issues.is_empty() {
            return Err(Error::Validation { issues });
        }
        let schema = A::Job::schema();
        let accepted: Vec<(String, Vec<String>)> = crate::class::dataset_fields(&schema, &document)
            .into_iter()
            .map(|(path, classes)| (catalogue_key(Path::new(&path)), classes))
            .collect();
        let mut inputs = Vec::new();
        for path in paths.iter().filter(|p| p.as_str() != output_dir) {
            let found: Option<(Id, String)> = self
                .db
                .query_row(
                    "SELECT id, class FROM datasets WHERE file = ?1 AND state = 'ready'",
                    [path],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(self.db_error())?;
            let Some((id, class)) = found else {
                issues.push(format!("{path} is not a dataset of this project"));
                continue;
            };
            if !inputs.contains(&id) {
                inputs.push(id);
            }
            for (_, classes) in accepted.iter().filter(|(p, _)| p == path) {
                if !classes.contains(&class) {
                    issues.push(format!(
                        "{path} is a {class} dataset; this field takes {}",
                        classes.join(" or ")
                    ));
                }
            }
            if let Err(e) = self.resolve_existing(Path::new(path)) {
                issues.push(e.to_string());
            }
        }
        if !issues.is_empty() {
            return Err(Error::Validation { issues });
        }
        Ok((document, job.resolve_paths(&self.root), inputs))
    }

    /// Check `document` as a job of analysis `A` in this project, without
    /// reading its inputs: types, and paths naming datasets of the classes
    /// their fields accept.
    pub fn check_job<A: Analysis>(&self, document: &serde_json::Value) -> Result<A::Job> {
        self.project_job::<A>(document, &format!("{RUNS_DIR}/new"))
            .map(|(_, job, _)| job)
    }

    /// Queue a run of `A` on `document`: its outputs will go to `runs/<id>`,
    /// tagged `tags` (the analysis name and run number when empty). The job
    /// and the tags are checked now; [`Project::execute`] runs it. Without
    /// `overwrite`, tags already taken by a dataset of an output class are
    /// refused; with it, that dataset is replaced once the run succeeds (AM5
    /// "overwrite warning").
    pub fn start_run<A: Analysis>(
        &self,
        document: &serde_json::Value,
        tags: &[String],
        overwrite: bool,
    ) -> Result<Id> {
        // Held while the run is recorded: a removal waits, or wins and this fails.
        let _hold = self.activity()?;
        // Fail early, before a run is recorded.
        self.check_job::<A>(document)?;
        self.db
            .execute(
                "INSERT INTO runs (analysis, tags, job, status, output_dir, started, overwrite)
                 VALUES (?1, '[]', ?2, 'queued', '', ?3, ?4)",
                params![A::NAME, json(document)?, now(), overwrite],
            )
            .map_err(self.db_error())?;
        let id = self.db.last_insert_rowid();
        let output_dir = format!("{RUNS_DIR}/{id}");
        let tags = if tags.is_empty() {
            super::normalize_tags(&[A::NAME.replace('-', "_"), format!("run{id}")])
        } else {
            super::normalize_tags(tags)
        };
        let checked = A::OUTPUTS
            .iter()
            .filter_map(|output| output.class)
            .try_for_each(|class| self.replaceable(class, &tags, overwrite, id).map(drop))
            .and_then(|()| self.project_job::<A>(document, &output_dir));
        let (stored, inputs) = match checked {
            Ok((stored, _, inputs)) => (stored, inputs),
            Err(error) => {
                self.close_run(id, RunStatus::Failed, None, Some(&error))?;
                return Err(error);
            }
        };
        let (tags, stored) = (json(&tags)?, json(&stored)?);
        // The job and the datasets it reads, together.
        let recorded = (|| {
            let tx = self.db.unchecked_transaction()?;
            tx.execute(
                "UPDATE runs SET tags = ?1, job = ?2, output_dir = ?3 WHERE id = ?4",
                params![tags, stored, output_dir, id],
            )?;
            if let Err(error) = link_inputs(&tx, id, &inputs)? {
                return Ok(Err(error));
            }
            tx.commit().map(Ok)
        })()
        .map_err(self.db_error())
        .and_then(|linked| linked);
        if let Err(error) = recorded {
            self.close_run(id, RunStatus::Failed, None, Some(&error))?;
            return Err(error);
        }
        Ok(id)
    }

    /// Run queued run `id` of `A`: prepare, run, register its outputs. A
    /// failed run is recorded and its error returned; a run cancelled before
    /// it starts is recorded as cancelled.
    pub fn execute<A: Analysis>(
        &self,
        id: Id,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        let record = self.run_record(id)?;
        if record.kind != RunKind::Analysis
            || record.analysis != A::NAME
            || record.status != RunStatus::Queued
        {
            return Err(Error::invalid(format!(
                "run {id} is a {} run, {}: it cannot run as {}",
                record.analysis,
                record.status.name(),
                A::NAME
            )));
        }
        let _hold = self.activity()?;
        let outcome = if monitor.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            let claimed = self
                .db
                .execute(
                    "UPDATE runs SET status = 'running', started = ?1
                     WHERE id = ?2 AND status = 'queued'",
                    params![now(), id],
                )
                .map_err(self.db_error())?;
            if claimed != 1 {
                // Cancelled or taken by another process meanwhile.
                return Err(Error::invalid(format!("run {id} is no longer queued")));
            }
            self.project_job::<A>(&record.job, &record.output_dir)
                .and_then(|(_, job, _)| A::prepare(job, settings, monitor))
                .and_then(|prepared| A::run(prepared, settings, monitor))
        };
        let finished = self.finish_run::<A>(
            id,
            outcome.as_ref().map(|(_, report)| report),
            settings,
            monitor,
        )?;
        outcome.map(|_| finished)
    }

    /// Record the end of run `id`: on success, register every output and
    /// mark the run done in one transaction, or neither.
    fn finish_run<A: Analysis>(
        &self,
        id: Id,
        result: std::result::Result<&A::Report, &Error>,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        let record = self.run_record(id)?;
        match result {
            Ok(report) => {
                monitor.stage("register");
                if let Err(error) = self.register_outputs::<A>(&record, report, settings, monitor) {
                    self.close_run(id, RunStatus::Failed, None, Some(&error))?;
                    return Err(error);
                }
            }
            Err(error) => {
                let status = if error.kind() == crate::error::ErrorKind::Cancelled {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                };
                self.close_run(id, status, None, Some(error))?;
            }
        }
        self.run_record(id)
    }

    /// Summarise the outputs of `record` (slow: overviews, fingerprints),
    /// then insert them and mark the run done atomically.
    fn register_outputs<A: Analysis>(
        &self,
        record: &RunRecord,
        report: &A::Report,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<()> {
        struct Ready {
            file: String,
            name: &'static str,
            class: crate::class::DataClass,
            summary: String,
            sha256: String,
            bytes: u64,
        }
        let mut ready: Vec<Ready> = Vec::new();
        let summarised = A::OUTPUTS
            .iter()
            .filter_map(|o| o.class.map(|class| (o.file, class)))
            .filter(|(name, _)| self.root.join(&record.output_dir).join(name).is_file())
            .try_for_each(|(name, class)| {
                let file = format!("{}/{name}", record.output_dir);
                let path = self.resolve(Path::new(&file))?;
                let summary = summarise(&path, class, settings, monitor.cancelled())?;
                let fingerprint =
                    crate::report::fingerprints(&[("output", &path)], monitor.cancelled())?
                        .pop()
                        .ok_or_else(|| Error::invalid("no fingerprint"))?;
                ready.push(Ready {
                    file,
                    name,
                    class,
                    summary: json(&summary)?,
                    sha256: fingerprint.sha256,
                    bytes: fingerprint.bytes,
                });
                Ok(())
            });
        let mut replaced: Vec<super::DatasetRecord> = Vec::new();
        let committed = summarised.and_then(|()| {
            self.db
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(self.db_error())?;
            let inserted = (|| {
                for output in &ready {
                    if let Some(old) = self.replaceable(
                        output.class,
                        &record.tags,
                        record.overwrite,
                        record.id,
                    )? {
                        // The run succeeded: the dataset it replaces is
                        // being deleted (its tags freed); its files go once
                        // this commits, or with `recover` after a crash.
                        self.db
                            .execute(
                                "UPDATE datasets SET state = 'deleting', tags_key = 'replaced-' || id
                                 WHERE id = ?1",
                                [old.id],
                            )
                            .map_err(self.db_error())?;
                        replaced.push(old);
                    }
                    let origin = Origin::Run {
                        run: record.id,
                        file: output.name.to_owned(),
                    };
                    self.db
                        .execute(
                            "INSERT INTO datasets (class, tags, tags_key, file, state, origin, run,
                             sha256, bytes, preview, created)
                             VALUES (?1, ?2, ?3, ?4, 'ready', ?5, ?6, ?7, ?8, ?9, ?10)",
                            params![
                                output.class.code(),
                                json(&record.tags)?,
                                record.tags.join("_"),
                                output.file,
                                json(&origin)?,
                                record.id,
                                output.sha256,
                                output.bytes as i64,
                                output.summary,
                                now()
                            ],
                        )
                        .map_err(self.db_error())?;
                }
                let mut outcome = A::outcome(report);
                for old in &replaced {
                    outcome.push_str(&format!(
                        "; replaced {} [{}] (dataset {})",
                        old.class.label(),
                        old.tags.join(" "),
                        old.id
                    ));
                }
                let done = self
                    .db
                    .execute(
                        "UPDATE runs SET status = 'done', outcome = ?1, error = NULL, finished = ?2
                         WHERE id = ?3 AND status = 'running'",
                        params![outcome, now(), record.id],
                    )
                    .map_err(self.db_error())?;
                if done != 1 {
                    return Err(Error::invalid(format!(
                        "run {} changed state while finishing",
                        record.id
                    )));
                }
                Ok(())
            })();
            let end = if inserted.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            };
            self.db.execute_batch(end).map_err(self.db_error())?;
            inserted
        });
        if committed.is_ok() {
            // The replaced datasets: files, then rows. What fails here stays
            // `deleting`, for `recover`.
            for old in &replaced {
                if self.remove_files(&old.file).is_ok() {
                    let _ = self
                        .db
                        .execute("DELETE FROM datasets WHERE id = ?1", [old.id]);
                }
            }
        }
        if committed.is_err() {
            // Nothing was registered: drop the overviews built for it.
            for output in &ready {
                if let Ok(path) = self.resolve(Path::new(&output.file)) {
                    let _ = std::fs::remove_file(crate::preview::overview_path(&path));
                }
            }
        }
        committed
    }

    fn close_run(
        &self,
        id: Id,
        status: RunStatus,
        outcome: Option<&str>,
        error: Option<&Error>,
    ) -> Result<()> {
        let error = error
            .map(|e| {
                json(&RunError {
                    kind: e.kind().name().to_owned(),
                    issues: e.issues(),
                })
            })
            .transpose()?;
        self.db
            .execute(
                "UPDATE runs SET status = ?1, outcome = ?2, error = ?3, finished = ?4
                 WHERE id = ?5 AND status IN ('queued', 'running')",
                params![status.name(), outcome, error, now(), id],
            )
            .map_err(self.db_error())?;
        Ok(())
    }

    /// Record run `id` as failed by an internal error (a panic caught by
    /// the caller), whatever its state.
    pub fn mark_failed(&self, id: Id, message: &str) -> Result<()> {
        let error = json(&RunError {
            kind: "internal".to_owned(),
            issues: vec![message.to_owned()],
        })?;
        self.db
            .execute(
                "UPDATE runs SET status = 'failed', error = ?1, finished = ?2 WHERE id = ?3",
                params![error, now(), id],
            )
            .map_err(self.db_error())?;
        Ok(())
    }

    /// Cancel run `id` while it is queued; `false` when it is no longer
    /// queued (a running run stops through its monitor).
    pub fn cancel_queued(&self, id: Id) -> Result<bool> {
        let error = json(&RunError {
            kind: crate::error::ErrorKind::Cancelled.name().to_owned(),
            issues: vec!["cancelled before it started".into()],
        })?;
        let changed = self
            .db
            .execute(
                "UPDATE runs SET status = 'cancelled', error = ?1, finished = ?2
                 WHERE id = ?3 AND status = 'queued'",
                params![error, now(), id],
            )
            .map_err(self.db_error())?;
        Ok(changed == 1)
    }

    /// Run analysis `A` on `document` now: [`Project::start_run`] then
    /// [`Project::execute`].
    pub fn run<A: Analysis>(
        &self,
        document: &serde_json::Value,
        tags: &[String],
        overwrite: bool,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        let id = self.start_run::<A>(document, tags, overwrite)?;
        self.execute::<A>(id, settings, monitor)
    }

    /// [`Project::start_run`] of the analysis named `analysis`.
    pub fn start_named(
        &self,
        analysis: &str,
        document: &serde_json::Value,
        tags: &[String],
        overwrite: bool,
    ) -> Result<Id> {
        struct Start<'a>(&'a Project, &'a serde_json::Value, &'a [String], bool);
        impl ForAnalysis for Start<'_> {
            type Output = Result<Id>;
            fn call<A: Analysis>(self) -> Result<Id> {
                self.0.start_run::<A>(self.1, self.2, self.3)
            }
        }
        by_name(analysis, Start(self, document, tags, overwrite))
            .unwrap_or_else(|| Err(unknown(analysis)))
    }

    /// [`Project::execute`] of queued run `id`, whatever its analysis.
    pub fn execute_queued(
        &self,
        id: Id,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        struct Execute<'a>(&'a Project, Id, &'a ExecutionSettings, Monitor<'a>);
        impl ForAnalysis for Execute<'_> {
            type Output = Result<RunRecord>;
            fn call<A: Analysis>(self) -> Result<RunRecord> {
                self.0.execute::<A>(self.1, self.2, self.3)
            }
        }
        let analysis = self.run_record(id)?.analysis;
        by_name(&analysis, Execute(self, id, settings, monitor))
            .unwrap_or_else(|| Err(unknown(&analysis)))
    }

    /// [`Project::check_job`] of the analysis named `analysis`: the quick
    /// check, without reading the inputs.
    pub fn check_named(&self, analysis: &str, document: &serde_json::Value) -> Result<()> {
        struct Check<'a>(&'a Project, &'a serde_json::Value);
        impl ForAnalysis for Check<'_> {
            type Output = Result<()>;
            fn call<A: Analysis>(self) -> Result<()> {
                self.0.check_job::<A>(self.1).map(drop)
            }
        }
        by_name(analysis, Check(self, document)).unwrap_or_else(|| Err(unknown(analysis)))
    }

    /// Validate `document` as a job of `analysis` against the inputs, without
    /// computing: the document `validate --json` prints.
    pub fn validate_named(
        &self,
        analysis: &str,
        document: &serde_json::Value,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<serde_json::Value> {
        struct Validate<'a>(
            &'a Project,
            &'a serde_json::Value,
            &'a ExecutionSettings,
            Monitor<'a>,
        );
        impl ForAnalysis for Validate<'_> {
            type Output = Result<serde_json::Value>;
            fn call<A: Analysis>(self) -> Result<serde_json::Value> {
                let job = self.0.check_job::<A>(self.1)?;
                let prepared = A::prepare(job, self.2, self.3)?;
                let mut document = A::validation(&prepared);
                document["status"] = serde_json::Value::from("valid");
                Ok(document)
            }
        }
        by_name(analysis, Validate(self, document, settings, monitor))
            .unwrap_or_else(|| Err(unknown(analysis)))
    }

    /// [`Project::run`] of the analysis named `analysis`.
    pub fn run_named(
        &self,
        analysis: &str,
        document: &serde_json::Value,
        tags: &[String],
        overwrite: bool,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        let id = self.start_named(analysis, document, tags, overwrite)?;
        self.execute_queued(id, settings, monitor)
    }

    /// Queue run `id` again, a failed, cancelled or interrupted analysis:
    /// a new run of the same analysis on the same job, tags and overwrite.
    pub fn retry_run(&self, id: Id) -> Result<Id> {
        let record = self.run_record(id)?;
        if record.kind != RunKind::Analysis {
            return Err(Error::invalid(format!(
                "run {id} is a {} operation: start it again from where it was started",
                record.analysis
            )));
        }
        if !matches!(
            record.status,
            RunStatus::Failed | RunStatus::Cancelled | RunStatus::Interrupted
        ) {
            return Err(Error::invalid(format!(
                "run {id} is {}: only a failed, cancelled or interrupted run is retried",
                record.status.name()
            )));
        }
        self.start_named(
            &record.analysis,
            &record.job,
            &record.tags,
            record.overwrite,
        )
    }

    /// Remove run `id`, neither queued nor running: its record and, when
    /// none of its outputs stays, its directory. With `outputs`, its output
    /// datasets are removed too; otherwise they stay, no longer linked to a
    /// run.
    pub fn remove_run(&self, id: Id, outputs: bool) -> Result<()> {
        let _hold = self.activity()?;
        let record = self.run_record(id)?;
        if matches!(record.status, RunStatus::Queued | RunStatus::Running) {
            return Err(Error::invalid(format!(
                "run {id} is {}: cancel it first",
                record.status.name()
            )));
        }
        if outputs {
            // Every output is checked before any is removed.
            for &dataset in &record.datasets {
                if let Some(reader) = self.active_reader(dataset, None)? {
                    return Err(Error::invalid(format!(
                        "output {dataset} of run {id} is read by run {reader}: remove the run                          once that run ends"
                    )));
                }
            }
            for &dataset in &record.datasets {
                self.remove_dataset(dataset)?;
            }
        }
        (|| {
            let tx = self.db.unchecked_transaction()?;
            tx.execute("UPDATE datasets SET run = NULL WHERE run = ?1", [id])?;
            tx.execute(
                "DELETE FROM runs WHERE id = ?1 AND status NOT IN ('queued', 'running')",
                [id],
            )?;
            tx.commit()
        })()
        .map_err(self.db_error())?;
        // Only the directory a run of this project owns, `runs/<id>`; a run
        // from a project file may name another.
        if record.output_dir == format!("{RUNS_DIR}/{id}") {
            let kept: i64 = self
                .db
                .query_row(
                    "SELECT COUNT(*) FROM datasets WHERE substr(file, 1, length(?1) + 1) = ?1 || '/'",
                    [&record.output_dir],
                    |r| r.get(0),
                )
                .map_err(self.db_error())?;
            if kept == 0 {
                let dir = self.resolve(Path::new(&record.output_dir))?;
                match std::fs::remove_dir_all(&dir) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(Error::file(&dir)(e)),
                }
            }
        }
        Ok(())
    }

    /// Queue operation `name` (a run of kind `manage`) with `params`,
    /// reading datasets `inputs`: while it is queued or running, they are
    /// neither removed nor replaced.
    pub fn start_operation(
        &self,
        name: &str,
        params: &serde_json::Value,
        inputs: &[Id],
    ) -> Result<Id> {
        let _hold = self.activity()?;
        let params = json(params)?;
        (|| {
            let tx = self.db.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO runs (analysis, tags, job, status, output_dir, started, kind)
                 VALUES (?1, '[]', ?2, 'queued', '', ?3, 'manage')",
                params![name, params, now()],
            )?;
            let id = tx.last_insert_rowid();
            if let Err(error) = link_inputs(&tx, id, inputs)? {
                return Ok(Err(error));
            }
            tx.commit().map(|()| Ok(id))
        })()
        .map_err(self.db_error())?
    }

    /// Claim queued operation `id` to run it now; `None` when it is no
    /// longer queued (cancelled meanwhile).
    pub fn claim_operation(&self, id: Id) -> Result<Option<RunRecord>> {
        let claimed = self
            .db
            .execute(
                "UPDATE runs SET status = 'running', started = ?1
                 WHERE id = ?2 AND status = 'queued' AND kind = 'manage'",
                params![now(), id],
            )
            .map_err(self.db_error())?;
        if claimed == 1 {
            self.run_record(id).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Record the end of running operation `id`: done with `outcome` and
    /// `result`, or failed (cancelled) with the error.
    pub fn finish_operation(
        &self,
        id: Id,
        outcome: std::result::Result<(String, serde_json::Value), &Error>,
    ) -> Result<RunRecord> {
        match outcome {
            Ok((outcome, result)) => {
                self.db
                    .execute(
                        "UPDATE runs SET status = 'done', outcome = ?1, result = ?2, finished = ?3
                         WHERE id = ?4 AND status = 'running'",
                        params![outcome, json(&result)?, now(), id],
                    )
                    .map_err(self.db_error())?;
            }
            Err(error) => {
                let status = if matches!(error, Error::Cancelled) {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                };
                self.close_run(id, status, None, Some(error))?;
            }
        }
        self.run_record(id)
    }

    /// Record operation `name`, already done, started at `started`: an
    /// operation that made this project (creation, import) and so ran
    /// before it existed.
    pub fn record_operation(
        &self,
        name: &str,
        params: &serde_json::Value,
        started: i64,
        outcome: &str,
        result: &serde_json::Value,
    ) -> Result<RunRecord> {
        self.db
            .execute(
                "INSERT INTO runs (analysis, tags, job, status, output_dir, outcome, started,
                 finished, kind, result)
                 VALUES (?1, '[]', ?2, 'done', '', ?3, ?4, ?5, 'manage', ?6)",
                params![name, json(params)?, outcome, started, now(), json(result)?],
            )
            .map_err(self.db_error())?;
        self.run_record(self.db.last_insert_rowid())
    }

    /// Runs, by identifier.
    pub fn runs(&self) -> Result<Vec<RunRecord>> {
        self.runs_of("SELECT id FROM runs ORDER BY id", [])
    }

    /// The `limit` latest runs, newest first.
    pub fn recent_runs(&self, limit: usize) -> Result<Vec<RunRecord>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.runs_of("SELECT id FROM runs ORDER BY id DESC LIMIT ?1", [limit])
    }

    /// Records of the run ids selected by `sql`.
    fn runs_of(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<RunRecord>> {
        let ids: Vec<Id> = {
            let mut stmt = self.db.prepare(sql).map_err(self.db_error())?;
            stmt.query_map(params, |r| r.get(0))
                .map_err(self.db_error())?
                .collect::<rusqlite::Result<_>>()
                .map_err(self.db_error())?
        };
        ids.into_iter().map(|id| self.run_record(id)).collect()
    }

    /// Run `id`.
    pub fn run_record(&self, id: Id) -> Result<RunRecord> {
        type Row = (
            Id,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
            Option<i64>,
        );
        let row: Option<Row> = self
            .db
            .query_row(
                "SELECT id, analysis, tags, job, status, output_dir, outcome, error, started,
                 finished FROM runs WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                    ))
                },
            )
            .optional()
            .map_err(self.db_error())?;
        let Some((id, analysis, tags, job, status, output_dir, outcome, error, started, finished)) =
            row
        else {
            return Err(Error::invalid(format!("no run {id} in this project")));
        };
        let (overwrite, kind, result): (bool, String, Option<String>) = self
            .db
            .query_row(
                "SELECT overwrite, kind, result FROM runs WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(self.db_error())?;
        let kind = match kind.as_str() {
            "manage" => RunKind::Manage,
            _ => RunKind::Analysis,
        };
        let datasets: Vec<Id> = {
            let mut stmt = self
                .db
                .prepare("SELECT id FROM datasets WHERE state = 'ready' AND run = ?1 ORDER BY id")
                .map_err(self.db_error())?;
            stmt.query_map([id], |r| r.get(0))
                .map_err(self.db_error())?
                .collect::<rusqlite::Result<_>>()
                .map_err(self.db_error())?
        };
        let inputs: Vec<Id> = {
            let mut stmt = self
                .db
                .prepare(
                    "SELECT d.id FROM run_inputs i JOIN datasets d ON d.id = i.dataset
                     WHERE i.run = ?1 AND d.state = 'ready' ORDER BY d.id",
                )
                .map_err(self.db_error())?;
            stmt.query_map([id], |r| r.get(0))
                .map_err(self.db_error())?
                .collect::<rusqlite::Result<_>>()
                .map_err(self.db_error())?
        };
        Ok(RunRecord {
            id,
            analysis,
            tags: decode(&tags, "run tags")?,
            job: decode(&job, "run job")?,
            status: RunStatus::from_code(&status)
                .ok_or_else(|| Error::invalid(format!("project database: run status {status}")))?,
            output_dir,
            outcome,
            error: error.map(|e| decode(&e, "run error")).transpose()?,
            started,
            finished,
            datasets,
            inputs,
            overwrite,
            kind,
            result: result.map(|r| decode(&r, "run result")).transpose()?,
        })
    }
}

/// Record datasets `inputs` as read by run `run`, each still ready: one
/// being removed is refused (the outer error is the database's).
fn link_inputs(
    tx: &rusqlite::Transaction<'_>,
    run: Id,
    inputs: &[Id],
) -> rusqlite::Result<Result<()>> {
    let mut inputs = inputs.to_vec();
    inputs.sort_unstable();
    inputs.dedup();
    for dataset in inputs {
        let linked = tx.execute(
            "INSERT INTO run_inputs (run, dataset)
             SELECT ?1, id FROM datasets WHERE id = ?2 AND state = 'ready'",
            [run, dataset],
        )?;
        if linked != 1 {
            return Ok(Err(Error::invalid(format!(
                "dataset {dataset} is no longer in the project"
            ))));
        }
    }
    Ok(Ok(()))
}

fn unknown(analysis: &str) -> Error {
    Error::invalid(format!("no analysis named {analysis:?}"))
}
