//! Runs of analyses inside a project.
//!
//! A run takes a job document whose paths are relative to the project. The
//! project sets its output directory (`runs/<id>`), checks that no path
//! leaves the project, runs the analysis exactly as the CLI does, and
//! registers the outputs as datasets tagged with the run's tags.

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};
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
}

/// Job document for analysis `A` with `output_dir` set: typed, every path
/// checked to stay in the project, then made absolute against `root`.
fn project_job<A: Analysis>(
    document: &serde_json::Value,
    output_dir: &str,
    root: &Path,
) -> Result<(serde_json::Value, A::Job)> {
    let mut document = document.clone();
    let Some(object) = document.as_object_mut() else {
        return Err(Error::invalid("a job document is a JSON object"));
    };
    object.insert("output_dir".into(), serde_json::Value::from(output_dir));
    let mut job = A::Job::from_json(&document.to_string())?;
    let mut issues = Vec::new();
    job.for_each_path(&mut |path: &mut PathBuf| {
        if let Err(issue) = check_relative(path) {
            issues.push(issue);
        }
    });
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    Ok((document, job.resolve_paths(root)))
}

impl Project {
    /// Check `document` as a job of analysis `A` in this project, without
    /// reading its inputs: types, and paths inside the project.
    pub fn check_job<A: Analysis>(&self, document: &serde_json::Value) -> Result<A::Job> {
        project_job::<A>(document, &format!("{RUNS_DIR}/new"), &self.root).map(|(_, job)| job)
    }

    /// Queue a run of `A` on `document`: its outputs will go to `runs/<id>`,
    /// tagged `tags` (the analysis name and run number when empty). The job
    /// and the tags are checked now; [`Project::execute`] runs it.
    pub fn start_run<A: Analysis>(
        &self,
        document: &serde_json::Value,
        tags: &[String],
    ) -> Result<Id> {
        // Fail early, before a run is recorded.
        self.check_job::<A>(document)?;
        self.db
            .execute(
                "INSERT INTO runs (analysis, tags, job, status, output_dir, started)
                 VALUES (?1, '[]', ?2, 'queued', '', ?3)",
                params![A::NAME, json(document)?, now()],
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
            .try_for_each(|class| self.free_tags(class, &tags, None).map(drop))
            .and_then(|()| project_job::<A>(document, &output_dir, &self.root));
        let stored = match checked {
            Ok((stored, _)) => stored,
            Err(error) => {
                self.close_run(id, RunStatus::Failed, None, Some(&error))?;
                return Err(error);
            }
        };
        self.db
            .execute(
                "UPDATE runs SET tags = ?1, job = ?2, output_dir = ?3 WHERE id = ?4",
                params![json(&tags)?, json(&stored)?, output_dir, id],
            )
            .map_err(self.db_error())?;
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
        if record.analysis != A::NAME || record.status != RunStatus::Queued {
            return Err(Error::invalid(format!(
                "run {id} is a {} run, {:?}: it cannot run as {}",
                record.analysis,
                record.status,
                A::NAME
            )));
        }
        let outcome = if monitor.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            self.db
                .execute(
                    "UPDATE runs SET status = 'running', started = ?1 WHERE id = ?2",
                    params![now(), id],
                )
                .map_err(self.db_error())?;
            project_job::<A>(&record.job, &record.output_dir, &self.root)
                .and_then(|(_, job)| A::prepare(job, settings, monitor))
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

    /// Record the end of run `id`: on success, register its outputs.
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
                let registered = A::OUTPUTS
                    .iter()
                    .filter_map(|o| o.class.map(|class| (o.file, class)))
                    .filter(|(file, _)| self.root.join(&record.output_dir).join(file).is_file())
                    .try_for_each(|(file, class)| {
                        self.register(
                            &format!("{}/{file}", record.output_dir),
                            class,
                            &record.tags,
                            &Origin::Run {
                                run: id,
                                file: file.to_owned(),
                            },
                            settings,
                            monitor.cancelled(),
                        )
                        .map(drop)
                    });
                if let Err(error) = registered {
                    self.close_run(id, RunStatus::Failed, None, Some(&error))?;
                    return Err(error);
                }
                self.close_run(id, RunStatus::Done, Some(&A::outcome(report)), None)?;
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
                "UPDATE runs SET status = ?1, outcome = ?2, error = ?3, finished = ?4 WHERE id = ?5",
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
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        let id = self.start_run::<A>(document, tags)?;
        self.execute::<A>(id, settings, monitor)
    }

    /// [`Project::start_run`] of the analysis named `analysis`.
    pub fn start_named(
        &self,
        analysis: &str,
        document: &serde_json::Value,
        tags: &[String],
    ) -> Result<Id> {
        struct Start<'a>(&'a Project, &'a serde_json::Value, &'a [String]);
        impl ForAnalysis for Start<'_> {
            type Output = Result<Id>;
            fn call<A: Analysis>(self) -> Result<Id> {
                self.0.start_run::<A>(self.1, self.2)
            }
        }
        by_name(analysis, Start(self, document, tags)).unwrap_or_else(|| Err(unknown(analysis)))
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
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        let id = self.start_named(analysis, document, tags)?;
        self.execute_queued(id, settings, monitor)
    }

    /// Runs, by identifier.
    pub fn runs(&self) -> Result<Vec<RunRecord>> {
        let ids: Vec<Id> = {
            let mut stmt = self
                .db
                .prepare("SELECT id FROM runs ORDER BY id")
                .map_err(self.db_error())?;
            stmt.query_map([], |r| r.get(0))
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
        let datasets: Vec<Id> = {
            let mut stmt = self
                .db
                .prepare(
                    "SELECT id FROM datasets WHERE state = 'ready'
                     AND json_extract(origin, '$.type') = 'run'
                     AND json_extract(origin, '$.run') = ?1 ORDER BY id",
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
        })
    }
}

fn unknown(analysis: &str) -> Error {
    Error::invalid(format!("no analysis named {analysis:?}"))
}
