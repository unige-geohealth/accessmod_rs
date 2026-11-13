//! The contract every analysis implements: a job document, validation
//! against the inputs (`prepare`), execution (`run`) and its `run.json`.
//! The CLI drives any analysis through it, and so will `accessmod serve`.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::class::DataClass;
use crate::error::Result;
use crate::job::JobFile;
use crate::preview::ValueCount;
use crate::report::Warning;
use crate::resources::ExecutionSettings;

/// Cancellation flag polled by long operations.
pub type Cancelled<'a> = &'a (dyn Fn() -> bool + Sync);

/// Progress of a running analysis, reported through a [`Monitor`].
///
/// Stages are stable identifiers, the keys of `durations_s` in `run.json`
/// (`load_inputs`, `solve`, `write_outputs`, …), so an interface can
/// translate them. Progress never changes results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Progress {
    /// Stage identifier.
    pub stage: String,
    /// Units done in the stage (cells, searches, facilities, iterations).
    pub done: u64,
    /// Units in the stage, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// What a running analysis polls and reports: cancellation and progress.
/// Both callbacks may be called from several threads.
#[derive(Clone, Copy)]
pub struct Monitor<'a> {
    cancelled: Cancelled<'a>,
    progress: &'a (dyn Fn(&Progress) + Sync),
}

impl std::fmt::Debug for Monitor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Monitor").finish_non_exhaustive()
    }
}

impl<'a> Monitor<'a> {
    /// Never cancelled; progress is dropped.
    pub const QUIET: Monitor<'static> = Monitor {
        cancelled: &|| false,
        progress: &|_| {},
    };

    /// A monitor over `cancelled` and `progress`.
    pub fn new(cancelled: Cancelled<'a>, progress: &'a (dyn Fn(&Progress) + Sync)) -> Self {
        Self {
            cancelled,
            progress,
        }
    }

    /// The cancellation flag, for toolkit functions that poll it.
    pub fn cancelled(&self) -> Cancelled<'a> {
        self.cancelled
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        (self.cancelled)()
    }

    /// A stage starts.
    pub fn stage(&self, stage: &str) {
        self.advance(stage, 0, None);
    }

    /// `done` units of `total` (when known) of `stage` are done.
    pub fn advance(&self, stage: &str, done: u64, total: Option<u64>) {
        (self.progress)(&Progress {
            stage: stage.to_owned(),
            done,
            total,
        });
    }
}

/// A file written by an analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    /// File name in the output directory.
    pub file: &'static str,
    /// Class it enters a project as; `None`: kept with the run only.
    pub class: Option<DataClass>,
}

/// What [`Analysis::lint`] may know of its inputs without reading a grid:
/// facts stored when they were imported.
pub trait LintContext {
    /// Cells per value of the integer raster at `path`, in ascending
    /// order; `None` when unknown (not a project dataset, too many values).
    fn class_counts(&self, path: &Path) -> Option<Vec<ValueCount>>;
}

/// A context that knows nothing: [`Analysis::lint`] checks only what it
/// can read cheaply.
#[derive(Debug, Clone, Copy)]
pub struct NoContext;

impl LintContext for NoContext {
    fn class_counts(&self, _path: &Path) -> Option<Vec<ValueCount>> {
        None
    }
}

/// Cells between two progress reports of a cell-by-cell stage.
pub const PROGRESS_CELLS: u64 = 1 << 16;

/// An analysis: job, validation, execution, report.
pub trait Analysis {
    /// Stable name: CLI command, API path, schema file (`travel-time`).
    const NAME: &'static str;
    /// Files the analysis may write in its output directory; optional
    /// outputs may be absent.
    const OUTPUTS: &'static [Output];
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
        monitor: Monitor<'_>,
    ) -> Result<Self::Prepared>;

    /// Cheap checks of a job whose paths are resolved: what [`prepare`]
    /// would find, at a cost per facility or per stored fact, never per
    /// cell of a grid. Fails with the problems `prepare` fails on; returns
    /// the warnings it would give. The quick check of an interface.
    ///
    /// [`prepare`]: Analysis::prepare
    fn lint(job: &Self::Job, context: &dyn LintContext) -> Result<Vec<Warning>> {
        let _ = (job, context);
        Ok(Vec::new())
    }

    /// Compute and write every output atomically; returns the output
    /// directory and the report.
    fn run(
        prepared: Self::Prepared,
        settings: &ExecutionSettings,
        monitor: Monitor<'_>,
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

/// Something done once per analysis type; see [`for_each`].
pub trait Visit {
    /// Called with analysis `A`.
    fn visit<A: Analysis>(&mut self);
}

/// Call `visitor` with every analysis, in a fixed order (toolbox first,
/// then analyses, as in AccessMod 5). The schema export, projects and the
/// server use this list; a CLI test checks every analysis has its command.
pub fn for_each(visitor: &mut impl Visit) {
    visitor.visit::<crate::merge::MergeLandCover>();
    visitor.visit::<crate::population::AdjustPopulation>();
    visitor.visit::<crate::travel_time::TravelTime>();
    visitor.visit::<crate::coverage::GeographicCoverage>();
    visitor.visit::<crate::referral::Referral>();
    visitor.visit::<crate::zonal::ZonalStatistics>();
    visitor.visit::<crate::scaling_up::ScalingUp>();
    visitor.visit::<crate::optimization::Optimization>();
}

/// A computation over one analysis type; see [`by_name`].
pub trait ForAnalysis {
    /// What it returns.
    type Output;
    /// Run it for analysis `A`.
    fn call<A: Analysis>(self) -> Self::Output;
}

/// Run `f` for the analysis named `name`; `None` when there is none.
pub fn by_name<F: ForAnalysis>(name: &str, f: F) -> Option<F::Output> {
    struct ByName<'a, F: ForAnalysis> {
        name: &'a str,
        f: Option<F>,
        output: Option<F::Output>,
    }
    impl<F: ForAnalysis> Visit for ByName<'_, F> {
        fn visit<A: Analysis>(&mut self) {
            if A::NAME == self.name
                && let Some(f) = self.f.take()
            {
                self.output = Some(f.call::<A>());
            }
        }
    }
    let mut visitor = ByName {
        name,
        f: Some(f),
        output: None,
    };
    for_each(&mut visitor);
    visitor.output
}

/// Name and job schema of every analysis, in [`for_each`] order.
pub fn job_schemas() -> Vec<(&'static str, serde_json::Value)> {
    struct Schemas(Vec<(&'static str, serde_json::Value)>);
    impl Visit for Schemas {
        fn visit<A: Analysis>(&mut self) {
            self.0.push((A::NAME, A::Job::schema()));
        }
    }
    let mut schemas = Schemas(Vec::new());
    for_each(&mut schemas);
    schemas.0
}

/// Classes of the datasets each analysis registers in a project, in
/// [`Analysis::OUTPUTS`] order: what a run with given tags may overwrite.
pub fn output_classes() -> Vec<(&'static str, Vec<crate::class::DataClass>)> {
    struct Outputs(Vec<(&'static str, Vec<crate::class::DataClass>)>);
    impl Visit for Outputs {
        fn visit<A: Analysis>(&mut self) {
            let classes = A::OUTPUTS.iter().filter_map(|o| o.class).collect();
            self.0.push((A::NAME, classes));
        }
    }
    let mut outputs = Outputs(Vec::new());
    for_each(&mut outputs);
    outputs.0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::class::{DataClass, SCHEMA_KEY, schema_classes};

    #[test]
    fn names_are_unique_kebab_case() {
        let names: Vec<&str> = job_schemas().iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), 8);
        assert_eq!(names.iter().collect::<BTreeSet<_>>().len(), names.len());
        for name in names {
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{name}"
            );
        }
    }

    #[test]
    fn every_dataset_field_names_known_classes() {
        for (name, schema) in job_schemas() {
            let found = schema_classes(&schema);
            assert!(!found.is_empty(), "{name}: no dataset field has a class");
            for (pointer, codes) in found {
                assert!(!codes.is_empty(), "{name} {pointer}: empty class list");
                for code in codes {
                    assert!(
                        DataClass::from_code(&code).is_some(),
                        "{name} {pointer}: unknown class {code}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_dem_field_is_a_dem() {
        for (name, schema) in job_schemas() {
            if let Some(dem) = schema["properties"].get("dem") {
                assert_eq!(dem[SCHEMA_KEY], serde_json::json!(["rDem"]), "{name}");
            }
        }
    }
}
