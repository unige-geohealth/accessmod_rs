//! The contract every analysis implements: a job document, validation
//! against the inputs (`prepare`), execution (`run`) and its `run.json`.
//! The CLI drives any analysis through it, and so will `accessmod serve`.

use std::path::PathBuf;

use serde::Serialize;

use crate::error::Result;
use crate::job::JobFile;
use crate::report::Warning;
use crate::resources::ExecutionSettings;

/// Cancellation flag polled by long operations.
pub type Cancelled<'a> = &'a (dyn Fn() -> bool + Sync);

/// An analysis: job, validation, execution, report.
pub trait Analysis {
    /// Job document.
    type Job: JobFile + Serialize;
    /// Inputs validated and loaded, ready to run.
    type Prepared;
    /// `run.json`.
    type Report: Serialize;

    /// Validate the job against its inputs and load them; fails with every
    /// problem found.
    fn prepare(
        job: Self::Job,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<Self::Prepared>;

    /// Compute and write every output atomically; returns the output
    /// directory and the report.
    fn run(
        prepared: Self::Prepared,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<(PathBuf, Self::Report)>;

    /// Non-blocking problems found while preparing.
    fn warnings(prepared: &Self::Prepared) -> &[Warning];

    /// One line describing prepared inputs, for logs.
    fn describe(prepared: &Self::Prepared) -> String;

    /// Document printed by `validate --json` (`status` is added).
    fn validation(prepared: &Self::Prepared) -> serde_json::Value;

    /// One line summarising a finished run, for logs.
    fn outcome(report: &Self::Report) -> String;
}
