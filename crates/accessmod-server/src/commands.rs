//! The command registry: every action that changes something.
//!
//! A command has an identifier, a parameter type (its JSON Schema is served
//! at `GET /api/commands`), the role it needs, and a function returning a
//! result or an error, never a panic. `POST /api/commands/{id}` dispatches
//! them; a future MCP server lists the same registry.

use std::path::Path;

use accessmod_io::class::DataClass;
use accessmod_io::project::{
    DatasetRecord, Id, ImportOptions, Project, ProjectInfo, RunRecord, SavedJob,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::{Principal, Requirement, Role};
use crate::error::{ApiError, ApiResult};
use crate::events::Event;
use crate::state::{AppState, new_slug};

/// Parameters naming the project they act on, if any.
pub trait Scoped {
    /// Project slug.
    fn project(&self) -> Option<&str>;
}

/// A command of the registry.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    /// Identifier, e.g. `dataset.import`.
    pub id: &'static str,
    /// What it does.
    pub description: &'static str,
    /// Who may run it.
    pub requirement: Requirement,
    /// JSON Schema of its parameters.
    pub params: fn() -> Value,
    /// Run it (blocking: SQLite, GDAL).
    pub run: fn(&AppState, &Principal, Value) -> ApiResult<Value>,
}

/// Listing entry of a command.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CommandInfo {
    /// Identifier.
    pub id: &'static str,
    /// What it does.
    pub description: &'static str,
    /// Who may run it.
    pub requirement: Requirement,
    /// JSON Schema of its parameters.
    pub params: Value,
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default()
}

/// Decode `params`, check the principal, run `f`, encode its result.
fn dispatch<P, R>(
    state: &AppState,
    principal: &Principal,
    requirement: Requirement,
    params: Value,
    f: fn(&AppState, &Principal, P) -> ApiResult<R>,
) -> ApiResult<Value>
where
    P: for<'de> Deserialize<'de> + Scoped,
    R: Serialize,
{
    let params: P = serde_json::from_value(params)
        .map_err(|e| ApiError::BadRequest(format!("invalid parameters: {e}")))?;
    principal.authorize(requirement, params.project())?;
    let result = f(state, principal, params)?;
    serde_json::to_value(result).map_err(|e| ApiError::Internal(format!("cannot encode: {e}")))
}

macro_rules! command {
    ($id:literal, $description:literal, $requirement:expr, $params:ty, $f:path) => {
        CommandSpec {
            id: $id,
            description: $description,
            requirement: $requirement,
            params: schema::<$params>,
            run: |state, principal, params| {
                dispatch::<$params, _>(state, principal, $requirement, params, $f)
            },
        }
    };
}

/// Every command.
pub fn registry() -> Vec<CommandSpec> {
    vec![
        command!(
            "project.create",
            "Create a project on the grid of a DEM",
            Requirement::User,
            CreateProject,
            create_project
        ),
        command!(
            "project.rename",
            "Change the name and description of a project",
            Requirement::Project(Role::Owner),
            RenameProject,
            rename_project
        ),
        command!(
            "dataset.import",
            "Import a file as a dataset of a class",
            Requirement::Project(Role::Editor),
            ImportDataset,
            import_dataset
        ),
        command!(
            "dataset.tag",
            "Replace the tags of a dataset",
            Requirement::Project(Role::Editor),
            TagDataset,
            tag_dataset
        ),
        command!(
            "dataset.remove",
            "Remove a dataset and its files",
            Requirement::Project(Role::Editor),
            DatasetRef,
            remove_dataset
        ),
        command!(
            "job.save",
            "Save a job document (a new one without id)",
            Requirement::Project(Role::Editor),
            SaveJob,
            save_job
        ),
        command!(
            "job.remove",
            "Remove a saved job",
            Requirement::Project(Role::Editor),
            JobRef,
            remove_job
        ),
        command!(
            "run.start",
            "Queue a run of an analysis on a job document",
            Requirement::Project(Role::Editor),
            StartRun,
            start_run
        ),
        command!(
            "run.cancel",
            "Cancel a queued or running run",
            Requirement::Project(Role::Editor),
            RunRef,
            cancel_run
        ),
    ]
}

/// Listing of the registry.
pub fn listing() -> Vec<CommandInfo> {
    registry()
        .into_iter()
        .map(|c| CommandInfo {
            id: c.id,
            description: c.description,
            requirement: c.requirement,
            params: (c.params)(),
        })
        .collect()
}

/// Run command `id` with `params`.
pub fn run(state: &AppState, principal: &Principal, id: &str, params: Value) -> ApiResult<Value> {
    let command = registry()
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("no command {id:?}")))?;
    (command.run)(state, principal, params)
}

// ---- parameters -------------------------------------------------------------

/// `project.create`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    /// Name.
    pub name: String,
    /// Free description.
    #[serde(default)]
    pub description: String,
    /// DEM defining the grid: a file on the server's disk (desktop).
    pub dem: String,
}

/// `project.rename`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameProject {
    /// Project slug.
    pub project: String,
    /// New name.
    pub name: String,
    /// New description.
    #[serde(default)]
    pub description: String,
}

/// `dataset.import`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportDataset {
    /// Project slug.
    pub project: String,
    /// File on the server's disk (desktop).
    pub path: String,
    /// Data class.
    pub class: DataClass,
    /// Tags naming the dataset.
    pub tags: Vec<String>,
    /// Layer of a vector file.
    #[serde(default)]
    pub layer: Option<String>,
}

/// `dataset.tag`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TagDataset {
    /// Project slug.
    pub project: String,
    /// Dataset.
    pub id: Id,
    /// New tags.
    pub tags: Vec<String>,
}

/// A dataset of a project.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetRef {
    /// Project slug.
    pub project: String,
    /// Dataset.
    pub id: Id,
}

/// `job.save`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveJob {
    /// Project slug.
    pub project: String,
    /// Saved job to replace; a new one when absent.
    #[serde(default)]
    pub id: Option<Id>,
    /// Analysis name.
    pub analysis: String,
    /// Free name.
    pub name: String,
    /// Job document, paths relative to the project (may be incomplete).
    pub document: Value,
}

/// A saved job of a project.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobRef {
    /// Project slug.
    pub project: String,
    /// Saved job.
    pub id: Id,
}

/// `run.start`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartRun {
    /// Project slug.
    pub project: String,
    /// Analysis name.
    pub analysis: String,
    /// Job document, paths relative to the project.
    pub document: Value,
    /// Tags of the datasets written (default: analysis name and run).
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A run of a project.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRef {
    /// Project slug.
    pub project: String,
    /// Run.
    pub id: Id,
}

impl Scoped for CreateProject {
    fn project(&self) -> Option<&str> {
        None
    }
}

macro_rules! scoped {
    ($($t:ty),+) => {$(
        impl Scoped for $t {
            fn project(&self) -> Option<&str> {
                Some(&self.project)
            }
        }
    )+};
}

scoped!(
    RenameProject,
    ImportDataset,
    TagDataset,
    DatasetRef,
    SaveJob,
    JobRef,
    StartRun,
    RunRef
);

// ---- handlers ---------------------------------------------------------------

/// A created project.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedProject {
    /// Slug, used in every project path.
    pub slug: String,
    /// The project.
    pub info: ProjectInfo,
}

fn create_project(
    state: &AppState,
    _: &Principal,
    params: CreateProject,
) -> ApiResult<CreatedProject> {
    let dem = state.local_path(&params.dem)?;
    let projects = state.projects_dir();
    let slug = new_slug(&projects, &params.name);
    let settings = &state.config().settings;
    let project = Project::create(
        &projects.join(&slug),
        &params.name,
        &params.description,
        Path::new(&dem),
        settings,
        &|| false,
    )?;
    let info = project.info()?;
    state.publish(Event::Projects);
    Ok(CreatedProject { slug, info })
}

fn rename_project(
    state: &AppState,
    _: &Principal,
    params: RenameProject,
) -> ApiResult<ProjectInfo> {
    let project = state.open(&params.project)?;
    project.rename(&params.name, &params.description)?;
    state.publish(Event::Projects);
    Ok(project.info()?)
}

fn import_dataset(
    state: &AppState,
    _: &Principal,
    params: ImportDataset,
) -> ApiResult<DatasetRecord> {
    let path = state.local_path(&params.path)?;
    let project = state.open(&params.project)?;
    let record = project.import(
        &path,
        params.class,
        &params.tags,
        &ImportOptions {
            layer: params.layer,
        },
        &state.config().settings,
        &|| false,
    )?;
    catalogue(state, &params.project);
    Ok(record)
}

fn tag_dataset(state: &AppState, _: &Principal, params: TagDataset) -> ApiResult<DatasetRecord> {
    let record = state
        .open(&params.project)?
        .retag(params.id, &params.tags)?;
    catalogue(state, &params.project);
    Ok(record)
}

fn remove_dataset(state: &AppState, _: &Principal, params: DatasetRef) -> ApiResult<()> {
    state.open(&params.project)?.remove_dataset(params.id)?;
    catalogue(state, &params.project);
    Ok(())
}

fn save_job(state: &AppState, _: &Principal, params: SaveJob) -> ApiResult<SavedJob> {
    let saved = state.open(&params.project)?.save_job(
        params.id,
        &params.analysis,
        &params.name,
        &params.document,
    )?;
    catalogue(state, &params.project);
    Ok(saved)
}

fn remove_job(state: &AppState, _: &Principal, params: JobRef) -> ApiResult<()> {
    state.open(&params.project)?.remove_job(params.id)?;
    catalogue(state, &params.project);
    Ok(())
}

fn start_run(state: &AppState, _: &Principal, params: StartRun) -> ApiResult<RunRecord> {
    let project = state.open(&params.project)?;
    let id = project.start_named(&params.analysis, &params.document, &params.tags)?;
    let record = project.run_record(id)?;
    state.runner().enqueue(&params.project, id);
    state.publish(Event::Run {
        project: params.project.clone(),
        run: record.clone(),
    });
    Ok(record)
}

fn cancel_run(state: &AppState, _: &Principal, params: RunRef) -> ApiResult<RunRecord> {
    let project = state.open(&params.project)?;
    let was_queued = project.cancel_queued(params.id)?;
    let signalled = state.runner().cancel(&params.project, params.id);
    let record = project.run_record(params.id)?;
    if !was_queued && !signalled {
        return Err(ApiError::Engine(accessmod_io::error::Error::invalid(
            format!(
                "run {} is {}: nothing to cancel",
                params.id,
                record.status.name()
            ),
        )));
    }
    state.publish(Event::Run {
        project: params.project,
        run: record.clone(),
    });
    Ok(record)
}

fn catalogue(state: &AppState, project: &str) {
    state.publish(Event::Catalogue {
        project: project.to_owned(),
    });
}
