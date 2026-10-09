//! `accessmod referral …`

use std::path::PathBuf;

use accessmod_io::referral::{PreparedReferral, ReferralJob, prepare, run as run_job};
use accessmod_io::travel_time::ExecutionSettings;
use clap::Subcommand;
use serde_json::json;

use crate::travel_time::{ExecArgs, fail, settings};
use crate::{Cancel, Failure, print_json};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the JSON schema of the referral job document.
    Schema,
    /// Validate a referral job against its inputs, without computing.
    Validate(Args),
    /// Validate and run a referral job.
    Run(Args),
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Referral job document (JSON).
    #[arg(long, value_name = "FILE")]
    job: PathBuf,
    #[command(flatten)]
    exec: ExecArgs,
}

fn load(
    args: &Args,
    settings: &ExecutionSettings,
    cancel: &Cancel,
) -> Result<PreparedReferral, Failure> {
    let json = args.exec.json;
    let job = ReferralJob::from_file(&args.job).map_err(|e| fail(&e, json))?;
    let prepared = prepare(job, settings, &|| cancel.requested()).map_err(|e| fail(&e, json))?;
    for warning in &prepared.warnings {
        tracing::warn!("{}", warning.message);
    }
    tracing::info!(
        "referral: {} origins x {} destinations on a {} x {} grid",
        prepared.origins.kept.len(),
        prepared.destinations.kept.len(),
        prepared.grid.cols,
        prepared.grid.rows
    );
    Ok(prepared)
}

pub fn run(command: Command, cancel: &Cancel) -> Result<(), Failure> {
    match command {
        Command::Schema => {
            print_json(&ReferralJob::json_schema());
            Ok(())
        }
        Command::Validate(args) => {
            let settings = settings(&args.exec)?;
            let prepared = load(&args, &settings, cancel)?;
            if args.exec.json {
                print_json(&json!({
                    "status": "valid",
                    "job": prepared.job,
                    "origins_used": prepared.origins.kept.len(),
                    "origins_skipped": prepared.origins.issues,
                    "destinations_used": prepared.destinations.kept.len(),
                    "destinations_skipped": prepared.destinations.issues,
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
                "wrote {}: {} pairs ({} reached) from {} searches{}",
                output_dir.display(),
                r.pairs,
                r.reached_pairs,
                r.searches,
                if r.reversed {
                    ", started from the destinations"
                } else {
                    ""
                }
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
