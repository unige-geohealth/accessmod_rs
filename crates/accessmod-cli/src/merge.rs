//! `accessmod merge-land-cover …`

use std::path::PathBuf;

use accessmod_io::merge::{MergeLandCoverJob, PreparedMerge, prepare, run as run_job};
use accessmod_io::travel_time::ExecutionSettings;
use clap::Subcommand;
use serde_json::json;

use crate::travel_time::{ExecArgs, fail, settings};
use crate::{Cancel, Failure, print_json};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the JSON schema of the merge job document.
    Schema,
    /// Validate a merge job against its inputs (classes, conflicts), without
    /// merging.
    Validate(Args),
    /// Validate and run a merge job.
    Run(Args),
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Merge job document (JSON).
    #[arg(long, value_name = "FILE")]
    job: PathBuf,
    #[command(flatten)]
    exec: ExecArgs,
}

fn load(
    args: &Args,
    settings: &ExecutionSettings,
    cancel: &Cancel,
) -> Result<PreparedMerge, Failure> {
    let json = args.exec.json;
    let job = MergeLandCoverJob::from_file(&args.job).map_err(|e| fail(&e, json))?;
    let prepared = prepare(job, settings, &|| cancel.requested()).map_err(|e| fail(&e, json))?;
    for warning in &prepared.warnings {
        tracing::warn!("{}", warning.message);
    }
    let classes: usize = prepared.items.iter().map(|i| i.classes.len()).sum();
    tracing::info!(
        "merge: {} stack items, {} classes on a {} x {} grid",
        prepared.items.len(),
        classes,
        prepared.grid.cols,
        prepared.grid.rows
    );
    Ok(prepared)
}

pub fn run(command: Command, cancel: &Cancel) -> Result<(), Failure> {
    match command {
        Command::Schema => {
            print_json(&MergeLandCoverJob::json_schema());
            Ok(())
        }
        Command::Validate(args) => {
            let settings = settings(&args.exec)?;
            let prepared = load(&args, &settings, cancel)?;
            if args.exec.json {
                print_json(&json!({
                    "status": "valid",
                    "job": prepared.job,
                    "stack": prepared.items,
                    "warnings": prepared.warnings,
                    "plan": prepared.plan,
                }));
            }
            Ok(())
        }
        Command::Run(args) => {
            let settings = settings(&args.exec)?;
            let json = args.exec.json;
            let prepared = load(&args, &settings, cancel)?;
            let (output_dir, report) =
                run_job(prepared, &settings, &|| cancel.requested()).map_err(|e| fail(&e, json))?;
            let r = &report.run.result;
            tracing::info!(
                "wrote {}: {} classified cells in {} classes, {} barrier cells, {} bridges removed",
                output_dir.display(),
                r.classified_cells,
                r.classes.len(),
                r.barrier_cells,
                r.bridges_removed
            );
            if json {
                let mut value = serde_json::to_value(&report).unwrap_or_default();
                value["status"] = json!("ok");
                value["output_dir"] = json!(output_dir);
                print_json(&value);
            }
            Ok(())
        }
    }
}
