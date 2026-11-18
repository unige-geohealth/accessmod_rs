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
    /// Create a project from a region and local source files: a recipe
    /// (JSON, `accessmod_io::creator::Recipe`; relative paths from the
    /// recipe's directory). DEM, land cover, population, OSM roads, rivers
    /// and facilities, merged land cover and a saved travel-time job.
    Create {
        /// Project directory (new or empty).
        dir: PathBuf,
        /// Recipe file.
        #[arg(long, value_name = "FILE")]
        recipe: PathBuf,
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
    /// Import a project file (`.am6p`) or an AccessMod 5 data archive
    /// (Data manager › Archive, `.zip`) as a new project directory.
    Import {
        /// Project file or AccessMod 5 data archive.
        file: PathBuf,
        /// New project directory (must not exist).
        dir: PathBuf,
        /// Project name (default: the one in the file, or the archive's
        /// file name).
        #[arg(long, default_value = "")]
        name: String,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Write the project to a new project file (`.am6p`) in a directory:
    /// datasets, runs and their jobs, with or without run outputs.
    Export {
        #[command(flatten)]
        project: ProjectArg,
        /// Directory receiving the file (created if missing).
        #[arg(long, value_name = "DIR")]
        dir: PathBuf,
        /// Leave run outputs out (their jobs are kept).
        #[arg(long)]
        no_results: bool,
        /// Print a JSON document on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Saved jobs of the project (made in the web interface or saved by
    /// `accessmod serve`): identifier, analysis, name and job document,
    /// paths relative to the project.
    Jobs {
        #[command(flatten)]
        project: ProjectArg,
        /// Print a JSON document on stdout.
        #[arg(long)]
        json: bool,
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
        Command::Create { dir, recipe, exec } => {
            let settings = settings(&exec)?;
            let text = std::fs::read_to_string(&recipe).map_err(|e| {
                fail(
                    &accessmod_io::error::Error::invalid(format!("{}: {e}", recipe.display())),
                    exec.json,
                )
            })?;
            let parsed: accessmod_io::creator::Recipe =
                serde_json::from_str(&text).map_err(|e| {
                    fail(
                        &accessmod_io::error::Error::invalid(format!("{}: {e}", recipe.display())),
                        exec.json,
                    )
                })?;
            let base = recipe.parent().unwrap_or_else(|| std::path::Path::new("."));
            let parsed = parsed.relative_to(base);
            let (project, manifest) = monitored(cancel, |m| {
                accessmod_io::creator::build(&parsed, &dir, &settings, m)
            })
            .map_err(|e| fail(&e, exec.json))?;
            let info = project.info().map_err(|e| fail(&e, exec.json))?;
            tracing::info!(
                "created {} in {} ({}; {} × {} cells)",
                info.name,
                dir.display(),
                manifest.crs_name,
                info.grid.cols,
                info.grid.rows
            );
            for warning in &manifest.warnings {
                tracing::warn!("{warning}");
            }
            if exec.json {
                print_value(&serde_json::json!({"project": info, "manifest": manifest}));
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
        Command::Import {
            file,
            dir,
            name,
            exec,
        } => {
            let settings = settings(&exec)?;
            let parent = dir
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            // A zip bomb fills at most 90 % of the free disk.
            let limits = accessmod_io::archive::Limits {
                bytes: accessmod_io::resources::free_disk(parent)
                    .map(|free| free / 10 * 9)
                    .unwrap_or(accessmod_io::archive::Limits::default().bytes),
                ..accessmod_io::archive::Limits::default()
            };
            let (project, summary) = monitored(cancel, |m| {
                Project::import_file(&file, &dir, name.trim(), limits, &settings, m)
            })
            .map_err(|e| fail(&e, exec.json))?;
            let info = project.info().map_err(|e| fail(&e, exec.json))?;
            tracing::info!("{summary}: {} in {}", info.name, dir.display());
            if exec.json {
                print_value(&serde_json::json!({"project": info, "summary": summary}));
            }
            Ok(())
        }
        Command::Export {
            project,
            dir,
            no_results,
            json,
        } => {
            let project = open(&project.project, json)?;
            std::fs::create_dir_all(&dir)
                .map_err(|e| fail(&Error::invalid(format!("{}: {e}", dir.display())), json))?;
            let file = monitored(cancel, |m| project.export_package(&dir, !no_results, m))
                .map_err(|e| fail(&e, json))?;
            tracing::info!("exported {}", file.display());
            if json {
                print_value(&serde_json::json!({ "file": file }));
            }
            Ok(())
        }
        Command::Jobs { project, json } => {
            let project = open(&project.project, json)?;
            let jobs = project.jobs().map_err(|e| fail(&e, json))?;
            if json {
                print_value(&jobs);
            } else {
                for j in &jobs {
                    println!("{:>4}  {:<30} {}", j.id, j.analysis, j.name);
                }
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
