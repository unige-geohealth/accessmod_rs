//! `accessmod project …` and the catalogue commands of `accessmod dataset …`:
//! projects, their datasets and runs (`accessmod_io::project`).

use std::path::{Path, PathBuf};

use accessmod_io::class::DataClass;
use accessmod_io::error::Error;
use accessmod_io::project::{DatasetRecord, ImportOptions, Project, RunRecord};
use serde::Serialize;

use crate::exec::{ExecArgs, fail, monitored, settings};
use crate::{Cancel, Failure, print_json};

/// Project directory option.
#[derive(Debug, clap::Args)]
pub struct ProjectArg {
    /// Project directory.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
}

/// `accessmod project …`.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Create a project in a new or empty directory, on the grid of a DEM.
    Init {
        /// Project directory.
        dir: PathBuf,
        /// DEM defining the grid (CRS, resolution, extent).
        #[arg(long, value_name = "FILE")]
        dem: PathBuf,
        /// Project name.
        #[arg(long)]
        name: String,
        /// Free description.
        #[arg(long, default_value = "")]
        description: String,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Create the demo project (AccessMod 5 sample data, tagged `demo`).
    Demo {
        /// Project directory.
        dir: PathBuf,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Name, grid and content of a project.
    Info {
        #[command(flatten)]
        project: ProjectArg,
        /// Print a JSON document on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Run an analysis on a job whose paths are relative to the project;
    /// outputs go to `runs/<id>` and enter the catalogue.
    Run {
        /// Analysis, e.g. `travel-time`.
        analysis: String,
        /// Job document (JSON); paths relative to the project.
        #[arg(long, value_name = "FILE")]
        job: PathBuf,
        /// Tags of the datasets written (default: analysis name and run).
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Replace the datasets of the output classes already tagged so, once
        /// the run succeeds (refused otherwise).
        #[arg(long)]
        overwrite: bool,
        #[command(flatten)]
        project: ProjectArg,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Runs of the project.
    Runs {
        #[command(flatten)]
        project: ProjectArg,
        /// Print a JSON document on stdout.
        #[arg(long)]
        json: bool,
    },
    /// After a crash: remove half-done imports and deletions, mark unfinished
    /// runs interrupted. Refused while an import or a run is in progress.
    Recover {
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Remove the project and every file in it. Refused while an import or a
    /// run is in progress or a run is queued.
    Remove {
        #[command(flatten)]
        project: ProjectArg,
    },
}

/// Catalogue subcommands of `accessmod dataset`.
#[derive(Debug, clap::Subcommand)]
pub enum DatasetCommand {
    /// Import a file into a project as a dataset of a class.
    Import {
        /// Raster, vector or CSV file.
        file: PathBuf,
        /// AccessMod 5 class, e.g. rLandCover, vFacility, tScenario.
        #[arg(long, value_parser = parse_class)]
        class: DataClass,
        /// Tags naming the dataset (with the class), e.g. `roads,2024`.
        #[arg(long, value_delimiter = ',', required = true)]
        tags: Vec<String>,
        /// Layer of a vector dataset (default: the first).
        #[arg(long)]
        layer: Option<String>,
        #[command(flatten)]
        project: ProjectArg,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Datasets of a project.
    List {
        /// Only this class.
        #[arg(long, value_parser = parse_class)]
        class: Option<DataClass>,
        #[command(flatten)]
        project: ProjectArg,
        /// Print a JSON document on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Replace the tags of a dataset.
    Tag {
        /// Dataset identifier.
        id: i64,
        /// New tags.
        #[arg(long, value_delimiter = ',', required = true)]
        tags: Vec<String>,
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Remove a dataset and its files.
    Remove {
        /// Dataset identifier.
        id: i64,
        #[command(flatten)]
        project: ProjectArg,
    },
}

fn parse_class(text: &str) -> Result<DataClass, String> {
    DataClass::from_code(text).ok_or_else(|| {
        let codes: Vec<&str> = DataClass::ALL.iter().map(|c| c.code()).collect();
        format!("unknown class {text:?}; one of {}", codes.join(", "))
    })
}

fn open(path: &Path, json: bool) -> Result<Project, Failure> {
    Project::open(path).map_err(|e| fail(&e, json))
}

fn print_value(value: &impl Serialize) {
    match serde_json::to_value(value) {
        Ok(value) => print_json(&value),
        Err(e) => eprintln!("error: cannot serialise output: {e}"),
    }
}

fn dataset_line(d: &DatasetRecord) -> String {
    format!(
        "{:>4}  {:<22} {:<28} {}",
        d.id,
        d.class.code(),
        d.tags.join(" "),
        d.file
    )
}

fn run_line(r: &RunRecord) -> String {
    let status = r.status.name();
    let detail = r
        .outcome
        .clone()
        .or_else(|| r.error.as_ref().map(|e| e.issues.join("; ")))
        .unwrap_or_default();
    format!("{:>4}  {:<30} {:<11} {detail}", r.id, r.analysis, status)
}

/// Run `accessmod project …`.
pub fn run(command: Command, cancel: &Cancel) -> Result<(), Failure> {
    match command {
        Command::Init {
            dir,
            dem,
            name,
            description,
            exec,
        } => {
            let settings = settings(&exec)?;
            let project = monitored(cancel, |m| {
                Project::create(&dir, &name, &description, &dem, &settings, m.cancelled())
            })
            .map_err(|e| fail(&e, exec.json))?;
            let info = project.info().map_err(|e| fail(&e, exec.json))?;
            tracing::info!(
                "created project {:?} in {}: {} x {} grid",
                info.name,
                dir.display(),
                info.grid.cols,
                info.grid.rows
            );
            if exec.json {
                print_value(&info);
            }
            Ok(())
        }
        Command::Demo { dir, exec } => {
            let settings = settings(&exec)?;
            let project = monitored(cancel, |m| {
                accessmod_io::demo::create(&dir, &settings, m.cancelled())
            })
            .map_err(|e| fail(&e, exec.json))?;
            let info = project.info().map_err(|e| fail(&e, exec.json))?;
            tracing::info!("created the demo project in {}", dir.display());
            if exec.json {
                print_value(&info);
            }
            Ok(())
        }
        Command::Info { project, json } => {
            let project = open(&project.project, json)?;
            let info = project.info().map_err(|e| fail(&e, json))?;
            let datasets = project.datasets(None).map_err(|e| fail(&e, json))?;
            let runs = project.runs().map_err(|e| fail(&e, json))?;
            if json {
                print_value(&serde_json::json!({
                    "project": info,
                    "datasets": datasets.len(),
                    "runs": runs.len(),
                }));
            } else {
                println!("{} — {}", info.name, info.description);
                println!(
                    "grid: {} x {} cells of {} m",
                    info.grid.cols, info.grid.rows, info.grid.geo_transform[1]
                );
                println!("datasets: {}, runs: {}", datasets.len(), runs.len());
            }
            Ok(())
        }
        Command::Run {
            analysis,
            job,
            tags,
            overwrite,
            project,
            exec,
        } => {
            let settings = settings(&exec)?;
            let project = open(&project.project, exec.json)?;
            let text = std::fs::read_to_string(&job).map_err(|e| {
                fail(
                    &Error::invalid(format!("{}: {e}", job.display())),
                    exec.json,
                )
            })?;
            let document: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                fail(
                    &Error::invalid(format!("{}: {e}", job.display())),
                    exec.json,
                )
            })?;
            let record = monitored(cancel, |m| {
                project.run_named(&analysis, &document, &tags, overwrite, &settings, m)
            })
            .map_err(|e| fail(&e, exec.json))?;
            tracing::info!(
                "run {} wrote {}: {}",
                record.id,
                record.output_dir,
                record.outcome.as_deref().unwrap_or("")
            );
            if exec.json {
                print_value(&record);
            }
            Ok(())
        }
        Command::Runs { project, json } => {
            let project = open(&project.project, json)?;
            let runs = project.runs().map_err(|e| fail(&e, json))?;
            if json {
                print_value(&runs);
            } else {
                for r in &runs {
                    println!("{}", run_line(r));
                }
            }
            Ok(())
        }
        Command::Recover { project } => {
            let project = open(&project.project, false)?;
            if project.recover().map_err(|e| fail(&e, false))? {
                tracing::info!("recovered {}", project.root().display());
                Ok(())
            } else {
                eprintln!(
                    "error: an import or a run is in progress in this project; try again later"
                );
                Err(Failure { code: 2 })
            }
        }
        Command::Remove { project } => {
            let project = open(&project.project, false)?;
            project.remove().map_err(|e| fail(&e, false))
        }
    }
}

/// Run a catalogue subcommand of `accessmod dataset`.
pub fn dataset(command: DatasetCommand, cancel: &Cancel) -> Result<(), Failure> {
    match command {
        DatasetCommand::Import {
            file,
            class,
            tags,
            layer,
            project,
            exec,
        } => {
            let settings = settings(&exec)?;
            let project = open(&project.project, exec.json)?;
            let record = monitored(cancel, |m| {
                project.import(
                    &file,
                    class,
                    &tags,
                    &ImportOptions { layer },
                    &settings,
                    m.cancelled(),
                )
            })
            .map_err(|e| fail(&e, exec.json))?;
            tracing::info!("imported {}", dataset_line(&record).trim());
            if exec.json {
                print_value(&record);
            }
            Ok(())
        }
        DatasetCommand::List {
            class,
            project,
            json,
        } => {
            let project = open(&project.project, json)?;
            let datasets = project.datasets(class).map_err(|e| fail(&e, json))?;
            if json {
                print_value(&datasets);
            } else {
                for d in &datasets {
                    println!("{}", dataset_line(d));
                }
            }
            Ok(())
        }
        DatasetCommand::Tag { id, tags, project } => {
            let project = open(&project.project, false)?;
            let record = project.retag(id, &tags).map_err(|e| fail(&e, false))?;
            println!("{}", dataset_line(&record));
            Ok(())
        }
        DatasetCommand::Remove { id, project } => {
            let project = open(&project.project, false)?;
            project.remove_dataset(id).map_err(|e| fail(&e, false))
        }
    }
}
