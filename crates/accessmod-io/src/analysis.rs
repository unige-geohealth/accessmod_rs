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
    /// Stable name: CLI command, API path, schema file (`travel-time`).
    const NAME: &'static str;
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

/// Something done once per analysis type; see [`for_each`].
pub trait Visit {
    /// Called with analysis `A`.
    fn visit<A: Analysis>(&mut self);
}

/// Call `visitor` with every analysis, in a fixed order (toolbox first,
/// then analyses, as in AccessMod 5). The CLI, the server and the schema
/// export share this list.
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
