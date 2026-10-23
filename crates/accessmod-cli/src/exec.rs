//! What every analysis command shares: execution options, error reporting,
//! the `schema` / `validate` / `run` subcommands of a job, and the report
//! printed by `run --json`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use accessmod_io::analysis::{Analysis, Monitor, Progress};
use accessmod_io::error::{Error, ErrorKind};
use accessmod_io::job::JobFile;
use accessmod_io::report::Warning;
use accessmod_io::resources::{self, ExecutionSettings};
use clap::Subcommand;
use serde::Serialize;
use serde_json::json;

use crate::{Cancel, Failure, print_json};

/// Subcommands of an analysis driven by a job file.
#[derive(Debug, Subcommand)]
pub enum JobCommand {
    /// Print the JSON schema of the job document.
    Schema,
    /// Validate a job against its inputs, without computing.
    Validate(JobArgs),
    /// Validate and run a job.
    Run(JobArgs),
}

/// A job file and execution options.
#[derive(Debug, clap::Args)]
pub struct JobArgs {
    /// Job document (JSON).
    #[arg(long, value_name = "FILE")]
    pub job: PathBuf,
    #[command(flatten)]
    pub exec: ExecArgs,
}

/// Execution options shared by analyses: they change speed, never results.
#[derive(Debug, clap::Args)]
pub struct ExecArgs {
    /// Print a JSON document on stdout.
    #[arg(long)]
    pub json: bool,
    /// Memory budget, e.g. 2G or 512M (default: half the available memory).
    #[arg(long, value_name = "SIZE")]
    pub memory: Option<String>,
    /// Threads for computation and GDAL compression (default: available
    /// CPUs, at most 16).
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
    /// Directory for scratch tiles (default: system temporary directory).
    #[arg(long, value_name = "DIR")]
    pub tmp_dir: Option<PathBuf>,
}

pub fn settings(args: &ExecArgs) -> Result<ExecutionSettings, Failure> {
    let mut settings = ExecutionSettings::from_host();
    if let Some(text) = &args.memory {
        settings.memory_bytes = resources::parse_size(text).ok_or_else(|| {
            fail(
                &Error::invalid(format!(
                    "--memory {text:?}: expected a size such as 2G or 512M"
                )),
                args.json,
            )
        })?;
    }
    if let Some(threads) = args.threads {
        if threads == 0 {
            return Err(fail(
                &Error::invalid("--threads must be at least 1"),
                args.json,
            ));
        }
        settings.threads = threads;
    }
    if let Some(dir) = &args.tmp_dir {
        settings.tmp_dir = dir.clone();
    }
    if !settings.tmp_dir.is_dir() {
        return Err(fail(
            &Error::invalid(format!(
                "--tmp-dir {} is not a directory",
                settings.tmp_dir.display()
            )),
            args.json,
        ));
    }
    Ok(settings)
}

fn kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Validation => "validation",
        ErrorKind::Io => "io",
        ErrorKind::Computation => "computation",
        ErrorKind::Cancelled => "cancelled",
    }
}

/// Report `error` on stderr (and stdout as JSON) and return its exit code.
pub fn fail(error: &Error, json: bool) -> Failure {
    let kind = error.kind();
    let issues: Vec<String> = match error {
        Error::Validation { issues } => issues.clone(),
        other => vec![other.to_string()],
    };
    match kind {
        ErrorKind::Validation => {
            eprintln!("error: invalid job");
            for issue in &issues {
                eprintln!("  - {issue}");
            }
        }
        ErrorKind::Cancelled => eprintln!("cancelled: no output was written"),
        _ => eprintln!("error: {error}"),
    }
    if json {
        print_json(&json!({
            "status": "error",
            "kind": kind_name(kind),
            "exit_code": kind.exit_code(),
            "issues": issues,
        }));
    }
    Failure {
        code: kind.exit_code(),
    }
}

/// Print a run report on stdout (`--json`), with `status` and `output_dir`.
fn print_report(report: &impl Serialize, output_dir: &Path, json: bool) {
    if json {
        let mut value = serde_json::to_value(report).unwrap_or_default();
        value["status"] = json!("ok");
        value["output_dir"] = json!(output_dir);
        print_json(&value);
    }
}

/// Log validation warnings on stderr.
fn log_warnings(warnings: &[Warning]) {
    for warning in warnings {
        tracing::warn!("{}", warning.message);
    }
}

/// Run a job subcommand of analysis `A`, the job read from a file.
pub fn job_command<A: Analysis>(command: JobCommand, cancel: &Cancel) -> Result<(), Failure> {
    match command {
        JobCommand::Schema => {
            print_json(&A::Job::schema());
            Ok(())
        }
        JobCommand::Validate(args) => {
            let job = A::Job::from_file(&args.job).map_err(|e| fail(&e, args.exec.json))?;
            validate::<A>(job, &args.exec, cancel)
        }
        JobCommand::Run(args) => {
            let job = A::Job::from_file(&args.job).map_err(|e| fail(&e, args.exec.json))?;
            execute::<A>(job, &args.exec, cancel)
        }
    }
}

/// Validate `job` against its inputs, without computing.
pub fn validate<A: Analysis>(job: A::Job, exec: &ExecArgs, cancel: &Cancel) -> Result<(), Failure> {
    let settings = settings(exec)?;
    let prepared = prepare::<A>(job, &settings, exec.json, cancel)?;
    if exec.json {
        let mut document = A::validation(&prepared);
        document["status"] = json!("valid");
        print_json(&document);
    }
    Ok(())
}

/// Validate and run `job`.
pub fn execute<A: Analysis>(job: A::Job, exec: &ExecArgs, cancel: &Cancel) -> Result<(), Failure> {
    let settings = settings(exec)?;
    let prepared = prepare::<A>(job, &settings, exec.json, cancel)?;
    let log = ProgressLog::default();
    let report_progress = |p: &Progress| log.report(p);
    let cancelled = || cancel.requested();
    let monitor = Monitor::new(&cancelled, &report_progress);
    let (output_dir, report) =
        A::run(prepared, &settings, monitor).map_err(|e| fail(&e, exec.json))?;
    tracing::info!("wrote {}: {}", output_dir.display(), A::outcome(&report));
    print_report(&report, &output_dir, exec.json);
    Ok(())
}

fn prepare<A: Analysis>(
    job: A::Job,
    settings: &ExecutionSettings,
    json: bool,
    cancel: &Cancel,
) -> Result<A::Prepared, Failure> {
    let log = ProgressLog::default();
    let report_progress = |p: &Progress| log.report(p);
    let cancelled = || cancel.requested();
    let monitor = Monitor::new(&cancelled, &report_progress);
    let prepared = A::prepare(job, settings, monitor).map_err(|e| fail(&e, json))?;
    log_warnings(A::warnings(&prepared));
    tracing::info!("{}", A::describe(&prepared));
    Ok(prepared)
}

/// Least time between two progress lines of one stage.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

/// Logs progress on stderr: each stage when it starts, then its counts at
/// most every [`PROGRESS_INTERVAL`].
#[derive(Debug, Default)]
struct ProgressLog {
    /// Current stage and when its last line was written.
    state: Mutex<Option<(String, Instant)>>,
}

impl ProgressLog {
    fn report(&self, progress: &Progress) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        match &mut *state {
            Some((stage, last)) if *stage == progress.stage => {
                if progress.done > 0 && now.duration_since(*last) >= PROGRESS_INTERVAL {
                    *last = now;
                    tracing::info!("{}", progress_line(progress));
                }
            }
            _ => {
                *state = Some((progress.stage.clone(), now));
                tracing::info!("{}", progress_line(progress));
            }
        }
    }
}

/// `solve: 1200 of 4800 (25 %)`, `solve: 1200`, or `solve` alone.
fn progress_line(progress: &Progress) -> String {
    match (progress.done, progress.total) {
        (0, _) => progress.stage.clone(),
        (done, Some(total)) if total > 0 => format!(
            "{}: {done} of {total} ({:.0} %)",
            progress.stage,
            done as f64 * 100.0 / total as f64
        ),
        (done, _) => format!("{}: {done}", progress.stage),
    }
}
