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

use super::{DatasetRecord, Id, Origin, Project, RUNS_DIR, check_relative, decode, json, now};
use crate::analysis::{Analysis, Monitor, Visit, for_each};
use crate::error::{Error, Result};
use crate::job::JobFile;
use crate::resources::ExecutionSettings;

/// State of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
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
    fn code(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Done => "done",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Interrupted => "interrupted",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        [
            RunStatus::Running,
            RunStatus::Done,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::Interrupted,
        ]
        .into_iter()
        .find(|s| s.code() == code)
    }
}

/// Why a run failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunError {
    /// `validation`, `io`, `computation` or `cancelled`.
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

/// A run started, its job ready for [`Analysis::prepare`].
#[derive(Debug)]
pub struct StartedRun<A: Analysis> {
    /// The run.
    pub id: Id,
    /// Job with absolute paths.
    pub job: A::Job,
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

    /// Record a new run of `A` on `document`: its outputs go to `runs/<id>`
    /// and are tagged `tags` (the analysis name and run number when empty).
    pub fn start_run<A: Analysis>(
        &self,
        document: &serde_json::Value,
        tags: &[String],
    ) -> Result<StartedRun<A>> {
        // Fail early, before a run is recorded.
        self.check_job::<A>(document)?;
        self.db
            .execute(
                "INSERT INTO runs (analysis, tags, job, status, output_dir, started)
                 VALUES (?1, '[]', ?2, 'running', '', ?3)",
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
        let prepared = (|| {
            for output in A::OUTPUTS {
                if let Some(class) = output.class {
                    self.free_tags(class, &tags, None)?;
                }
            }
            project_job::<A>(document, &output_dir, &self.root)
        })();
        let (stored, job) = match prepared {
            Ok(ready) => ready,
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
        Ok(StartedRun { id, job })
    }

    /// Record the end of run `id`: on success, register its outputs.
    pub fn finish_run<A: Analysis>(
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
                    .map(|(file, class)| {
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
                    })
                    .collect::<Result<Vec<DatasetRecord>>>();
                match registered {
                    Ok(_) => {
                        self.close_run(id, RunStatus::Done, Some(&A::outcome(report)), None)?
                    }
                    Err(error) => {
                        self.close_run(id, RunStatus::Failed, None, Some(&error))?;
                        return Err(error);
                    }
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
                params![status.code(), outcome, error, now(), id],
            )
            .map_err(self.db_error())?;
        Ok(())
    }

    /// Run analysis `A` on `document` in this project: start, prepare, run,
    /// finish. A failed run is recorded and its error returned.
    pub fn run<A: Analysis>(
        &self,
        document: &serde_json::Value,
        tags: &[String],
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
    ) -> Result<RunRecord> {
        let started = self.start_run::<A>(document, tags)?;
        let outcome = A::prepare(started.job, settings, monitor)
            .and_then(|prepared| A::run(prepared, settings, monitor));
        let record = self.finish_run::<A>(
            started.id,
            outcome.as_ref().map(|(_, report)| report),
            settings,
            monitor,
        )?;
        outcome.map(|_| record)
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
        struct Runner<'a> {
            project: &'a Project,
            analysis: &'a str,
            document: &'a serde_json::Value,
            tags: &'a [String],
            settings: &'a ExecutionSettings,
            monitor: Monitor<'a>,
            result: Option<Result<RunRecord>>,
        }
        impl Visit for Runner<'_> {
            fn visit<A: Analysis>(&mut self) {
                if A::NAME == self.analysis {
                    self.result = Some(self.project.run::<A>(
                        self.document,
                        self.tags,
                        self.settings,
                        self.monitor,
                    ));
                }
            }
        }
        let mut runner = Runner {
            project: self,
            analysis,
            document,
            tags,
            settings,
            monitor,
            result: None,
        };
        for_each(&mut runner);
        runner
            .result
            .unwrap_or_else(|| Err(Error::invalid(format!("no analysis named {analysis:?}"))))
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
