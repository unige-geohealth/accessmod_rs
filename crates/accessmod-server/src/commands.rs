//! The command registry: every action that changes something.
//!
//! A command has an identifier, a parameter type (its JSON Schema is served
//! at `GET /api/commands`), the role it needs, and a function returning a
//! result or an error, never a panic. `POST /api/commands/{id}` dispatches
//! them; a future MCP server lists the same registry.

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
use crate::operations::{self, PendingOperation};
use crate::runner::Cancel;
use crate::settings::Settings;
use crate::state::{AppState, DEMO_SLUG};
use crate::uploads::{self, FileSource};

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
            "creator.countries",
            "Countries a project can be created in (geoBoundaries)",
            Requirement::User,
            crate::creator::Countries,
            crate::creator::countries
        ),
        command!(
            "creator.units",
            "Administrative units of a country at a level, simplified, to pick a region",
            Requirement::User,
            crate::creator::Units,
            crate::creator::units
        ),
        command!(
            "creator.create",
            "Create a project from a region: download its sources (DEM, land cover, population, OpenStreetMap) and build it",
            Requirement::User,
            crate::creator::CreateFromRegion,
            crate::creator::create
        ),
        command!(
            "project.rename",
            "Change the name and description of a project",
            Requirement::Project(Role::Owner),
            RenameProject,
            rename_project
        ),
        command!(
            "project.export",
            "Write the project to a project file (.am6p), to download",
            Requirement::Project(Role::Viewer),
            ExportProject,
            export_project
        ),
        command!(
            "project.import",
            "Create a project from a project file (.am6p) or an AccessMod 5 data archive (.zip)",
            Requirement::User,
            ImportProject,
            import_project
        ),
        command!(
            "project.remove",
            "Remove a project and every file in it",
            Requirement::Project(Role::Owner),
            ProjectRef,
            remove_project
        ),
        command!(
            "demo.reset",
            "Build the demo project again from the AccessMod 5 sample data",
            Requirement::Admin,
            ResetDemo,
            reset_demo
        ),
        command!(
            "settings.default_project",
            "Set or clear the project the client opens first",
            Requirement::Admin,
            DefaultProject,
            default_project
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
            "dataset.relocate",
            "A new facilities dataset with some facilities moved (interactive map)",
            Requirement::Project(Role::Editor),
            RelocateFacilities,
            relocate_facilities
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
            "archive.create",
            "Archive datasets and the jobs of runs in a zip of the project (Data › Archive)",
            Requirement::Project(Role::Editor),
            CreateArchive,
            create_archive
        ),
        command!(
            "archive.remove",
            "Remove an archive of the project",
            Requirement::Project(Role::Editor),
            ArchiveRef,
            remove_archive
        ),
        command!(
            "run.cancel",
            "Cancel a queued or running run",
            Requirement::Project(Role::Editor),
            RunRef,
            cancel_run
        ),
        command!(
            "run.pause",
            "Hold a queued or running run where it is, until resumed",
            Requirement::Project(Role::Editor),
            RunRef,
            pause_run
        ),
        command!(
            "run.resume",
            "Let a paused run go on",
            Requirement::Project(Role::Editor),
            RunRef,
            resume_run
        ),
        command!(
            "run.retry",
            "Queue a failed, cancelled or interrupted analysis again, on the same job",
            Requirement::Project(Role::Editor),
            RunRef,
            retry_run
        ),
        command!(
            "operation.cancel",
            "Cancel an operation making a project (creation, import, demo)",
            Requirement::User,
            OperationRef,
            cancel_operation
        ),
        command!(
            "run.remove",
            "Remove a finished run; its outputs too on request",
            Requirement::Project(Role::Editor),
            RemoveRun,
            remove_run
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
    /// DEM defining the grid: an upload, or a file on the server's disk
    /// (desktop).
    pub dem: FileSource,
}

/// `project.export`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportProject {
    /// Project slug.
    pub project: String,
    /// With run outputs; without, runs keep their jobs only.
    #[serde(default = "yes")]
    pub results: bool,
}

fn yes() -> bool {
    true
}

/// A project file written by `project.export`, at
/// `GET /api/projects/{project}/exports/{name}`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportedProject {
    /// File name.
    pub name: String,
    /// Size, bytes.
    pub bytes: u64,
}

/// `project.import`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportProject {
    /// Name of the new project; the file's when empty.
    #[serde(default)]
    pub name: String,
    /// The project file (`.am6p`) or AccessMod 5 data archive (`.zip`):
    /// an upload, or a file on the server's disk (desktop).
    pub package: FileSource,
}

/// A project.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectRef {
    /// Project slug.
    pub project: String,
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
    /// File to import: an upload, or a file on the server's disk (desktop).
    pub source: FileSource,
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

/// `dataset.relocate`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelocateFacilities {
    /// Project slug.
    pub project: String,
    /// Facilities dataset; it stays as it is.
    pub id: Id,
    /// Facilities moved: feature id and new position (WGS 84).
    pub moves: Vec<accessmod_io::vector::PointMove>,
    /// Tags of the new dataset.
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

/// `archive.create`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateArchive {
    /// Project slug.
    pub project: String,
    /// Prefix of the file name (letters, digits, - and _); `am6_export` when empty.
    #[serde(default)]
    pub name: String,
    /// Datasets to archive.
    #[serde(default)]
    pub datasets: Vec<Id>,
    /// Runs whose jobs to archive.
    #[serde(default)]
    pub runs: Vec<Id>,
}

/// An archive of a project.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRef {
    /// Project slug.
    pub project: String,
    /// File name, as listed.
    pub name: String,
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
    /// Revision of the saved job the client edited (with `id`).
    #[serde(default)]
    pub revision: Option<i64>,
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
    /// Replace the datasets of the output classes already tagged so, once
    /// the run succeeds; refused otherwise.
    #[serde(default)]
    pub overwrite: bool,
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

/// An operation making a project.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationRef {
    /// Operation, as `GET /api/operations` lists it.
    pub id: Id,
}

impl Scoped for OperationRef {
    fn project(&self) -> Option<&str> {
        None
    }
}

/// `run.remove`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveRun {
    /// Project slug.
    pub project: String,
    /// Run.
    pub id: Id,
    /// Remove its output datasets too; otherwise they stay, no longer linked
    /// to a run.
    #[serde(default)]
    pub outputs: bool,
}

/// `demo.reset`: no parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResetDemo {}

/// `settings.default_project`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultProject {
    /// Project slug; `null` clears the setting (the demo opens first).
    pub project: Option<String>,
}

impl Scoped for CreateProject {
    fn project(&self) -> Option<&str> {
        None
    }
}

impl Scoped for ImportProject {
    fn project(&self) -> Option<&str> {
        None
    }
}

impl Scoped for ResetDemo {
    fn project(&self) -> Option<&str> {
        None
    }
}

impl Scoped for DefaultProject {
    fn project(&self) -> Option<&str> {
        self.project.as_deref()
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
    ProjectRef,
    ExportProject,
    RenameProject,
    ImportDataset,
    TagDataset,
    RelocateFacilities,
    DatasetRef,
    SaveJob,
    JobRef,
    StartRun,
    RunRef,
    RemoveRun,
    CreateArchive,
    ArchiveRef
);

// ---- handlers ---------------------------------------------------------------

fn create_project(
    state: &AppState,
    principal: &Principal,
    params: CreateProject,
) -> ApiResult<PendingOperation> {
    let dem = uploads::resolve(state, principal, &params.dem)?;
    let slug = free_slug(state, &params.name);
    let job = serde_json::json!({ "name": params.name, "description": params.description });
    let (target, name) = (slug.clone(), params.name.clone());
    operations::queue_pending(
        state,
        principal,
        "project-create",
        &slug,
        &name,
        job,
        move |state, monitor| {
            monitor.stage("import");
            let project = Project::create(
                &state.projects_dir().join(&target),
                &params.name,
                &params.description,
                &dem.path,
                &state.config().settings,
                monitor.cancelled(),
            )?;
            let outcome = format!("created on the grid of {}", file_label(&dem.path));
            dem.consumed();
            Ok(Some((project, outcome)))
        },
    )
}

/// A slug for a new project named `name`: free on disk and not reserved by
/// an operation making a project.
fn free_slug(state: &AppState, name: &str) -> String {
    crate::state::new_slug_except(&state.projects_dir(), name, &|slug| {
        state.pending().reserves(slug)
    })
}

/// The name of a file, for outcomes.
fn file_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn export_project(state: &AppState, _: &Principal, params: ExportProject) -> ApiResult<RunRecord> {
    let job = serde_json::json!({ "results": params.results });
    operations::queue_in_project(
        state,
        &params.project,
        "project-export",
        &job,
        &[],
        |_, _| {},
        move |project, monitor| {
            let name = project.export_latest(params.results, monitor)?;
            let bytes = std::fs::metadata(project.exported_package(&name)?)
                .map(|m| m.len())
                .unwrap_or(0);
            Ok((
                format!("exported to {name}"),
                ExportedProject { name, bytes },
            ))
        },
    )
}

fn import_project(
    state: &AppState,
    principal: &Principal,
    params: ImportProject,
) -> ApiResult<PendingOperation> {
    use accessmod_io::am5;
    use accessmod_io::archive::{self, Limits};
    let package = uploads::resolve_archive(state, principal, &params.package)?;
    let projects = state.projects_dir();
    std::fs::create_dir_all(&projects)
        .map_err(|e| ApiError::Internal(format!("{}: {e}", projects.display())))?;
    // A zip bomb fills at most 90 % of the free disk.
    let limits = Limits {
        bytes: accessmod_io::resources::free_disk(&projects)
            .map(|free| free / 10 * 9)
            .unwrap_or(Limits::default().bytes),
        ..Limits::default()
    };
    let file = file_label(&package.path);
    let rename = params.name.trim().to_owned();
    // An AccessMod 5 data archive (Data manager › Archive) has no manifest
    // and a DEM directory; it takes the archive's name. A project file's
    // manifest is read now: a file that is neither fails at once.
    let (name, import_name) = if am5::is_archive(&archive::file_names(&package.path, limits)?) {
        let name = if rename.is_empty() {
            am5::project_name(&package.path)
        } else {
            rename
        };
        (name.clone(), name)
    } else {
        let manifest = archive::manifest(&package.path, limits)?;
        let from_file = manifest
            .content
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let name = if rename.is_empty() {
            from_file
        } else {
            rename.clone()
        };
        (name, rename)
    };
    let slug = free_slug(state, &name);
    let target = slug.clone();
    let job = serde_json::json!({ "name": name, "file": file });
    operations::queue_pending(
        state,
        principal,
        "project-import",
        &slug,
        &name,
        job,
        move |state, monitor| {
            let (project, summary) = Project::import_file(
                &package.path,
                &state.projects_dir().join(&target),
                &import_name,
                limits,
                &state.config().settings,
                monitor,
            )?;
            package.consumed();
            Ok(Some((project, format!("{summary} from {file}"))))
        },
    )
}

/// `project.remove`: a job like the others (notified, listed in Jobs); no
/// project records it, so the journal keeps it.
fn remove_project(
    state: &AppState,
    principal: &Principal,
    params: ProjectRef,
) -> ApiResult<PendingOperation> {
    // The project as it is now: one made again at its path since (a demo
    // reset queued before, a restore from the command line) is another.
    let project = state.open(&params.project)?;
    let (name, instance) = (project.info()?.name, project.instance()?);
    let slug = params.project.clone();
    operations::queue_pending(
        state,
        principal,
        "project-remove",
        &params.project,
        &name,
        serde_json::json!({ "project": params.project }),
        move |state, _| {
            state.open(&slug)?.remove_if(&instance)?;
            // Removed: a setting left behind is a warning, not a failed removal.
            let cleared = state.update_settings(|s| {
                if s.default_project.as_deref() == Some(slug.as_str()) {
                    s.default_project = None;
                }
                Ok(())
            });
            if let Err(e) = cleared {
                tracing::warn!("project {slug} removed, default project not cleared: {e}");
            }
            Ok(None)
        },
    )
}

fn reset_demo(
    state: &AppState,
    principal: &Principal,
    _: ResetDemo,
) -> ApiResult<PendingOperation> {
    // Refused at once when the slug holds a user project (checked again
    // when it runs).
    if let Ok(project) = state.open(DEMO_SLUG)
        && !project.info()?.demo
    {
        return Err(ApiError::BadRequest(format!(
            "project {DEMO_SLUG:?} is not the demo project: remove it first"
        )));
    }
    operations::queue_pending(
        state,
        principal,
        "demo-reset",
        DEMO_SLUG,
        "Demo",
        serde_json::json!({}),
        |state, monitor| {
            state.reset_demo(monitor.cancelled())?;
            Ok(Some((
                state.open(DEMO_SLUG)?,
                "built again from the AccessMod 5 sample data".to_owned(),
            )))
        },
    )
}

fn default_project(state: &AppState, _: &Principal, params: DefaultProject) -> ApiResult<Settings> {
    // Checked with the settings held: a removal clears the setting after
    // the project is gone, so it cannot keep a removed project.
    state.update_settings(|s| {
        if let Some(slug) = &params.project {
            state.project_dir(slug)?;
        }
        s.default_project = params.project;
        Ok(())
    })
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
    principal: &Principal,
    params: ImportDataset,
) -> ApiResult<RunRecord> {
    let source = uploads::resolve(state, principal, &params.source)?;
    let file = file_label(&source.path);
    let job = serde_json::json!({
        "file": file,
        "class": params.class,
        "tags": params.tags,
        "layer": params.layer,
    });
    let settings = state.config().settings.clone();
    operations::queue_in_project(
        state,
        &params.project,
        "dataset-import",
        &job,
        &[],
        catalogue,
        move |project, monitor| {
            monitor.stage("import");
            let record = project.import(
                &source.path,
                params.class,
                &params.tags,
                &ImportOptions {
                    layer: params.layer,
                },
                &settings,
                monitor.cancelled(),
            )?;
            source.consumed();
            let mut outcome = format!(
                "{file} imported as dataset {} ({} {})",
                record.id,
                record.class.label(),
                record.tags.join(" ")
            );
            if let Some(conversion) = record.conversion() {
                outcome.push_str("; ");
                outcome.push_str(&conversion);
            }
            Ok((outcome, record))
        },
    )
}

fn tag_dataset(state: &AppState, _: &Principal, params: TagDataset) -> ApiResult<DatasetRecord> {
    let record = state
        .open(&params.project)?
        .retag(params.id, &params.tags)?;
    catalogue(state, &params.project);
    Ok(record)
}

fn relocate_facilities(
    state: &AppState,
    _: &Principal,
    params: RelocateFacilities,
) -> ApiResult<DatasetRecord> {
    let record = state.open(&params.project)?.relocate(
        params.id,
        &params.moves,
        &params.tags,
        &state.config().settings,
        &|| false,
    )?;
    catalogue(state, &params.project);
    Ok(record)
}

fn remove_dataset(state: &AppState, _: &Principal, params: DatasetRef) -> ApiResult<()> {
    state.open(&params.project)?.remove_dataset(params.id)?;
    catalogue(state, &params.project);
    Ok(())
}

fn create_archive(state: &AppState, _: &Principal, params: CreateArchive) -> ApiResult<RunRecord> {
    if params.datasets.is_empty() && params.runs.is_empty() {
        return Err(ApiError::BadRequest(
            "nothing to archive: select datasets or jobs".into(),
        ));
    }
    let job = serde_json::json!({
        "name": params.name,
        "datasets": params.datasets,
        "runs": params.runs,
    });
    let (slug, inputs) = (params.project.clone(), params.datasets.clone());
    operations::queue_in_project(
        state,
        &slug,
        "archive",
        &job,
        &inputs,
        |_, _| {},
        move |project, monitor| {
            let info =
                project.create_archive(&params.name, &params.datasets, &params.runs, monitor)?;
            Ok((format!("archived as {}", info.name), info))
        },
    )
}

fn remove_archive(state: &AppState, _: &Principal, params: ArchiveRef) -> ApiResult<()> {
    Ok(state.open(&params.project)?.remove_archive(&params.name)?)
}

fn save_job(state: &AppState, _: &Principal, params: SaveJob) -> ApiResult<SavedJob> {
    let target = match (params.id, params.revision) {
        (Some(id), Some(revision)) => Some((id, revision)),
        (None, _) => None,
        (Some(_), None) => {
            return Err(ApiError::BadRequest(
                "saving over a job needs the revision that was read".into(),
            ));
        }
    };
    let saved = state.open(&params.project)?.save_job(
        target,
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
    let id = project.start_named(
        &params.analysis,
        &params.document,
        &params.tags,
        params.overwrite,
    )?;
    queue_analysis(state, &project, &params.project, id)
}

/// Publish queued analysis run `id` and hand it to the compute lane.
fn queue_analysis(state: &AppState, project: &Project, slug: &str, id: Id) -> ApiResult<RunRecord> {
    let params = RunRef {
        project: slug.to_owned(),
        id,
    };
    let record = project.run_record(id)?;
    // Announce the queued run before the worker may start it, so its events
    // arrive in order.
    state.publish(Event::Run {
        project: params.project.clone(),
        run: Box::new(record.clone()),
    });
    if let Err(e) = state.runner().enqueue(&params.project, id) {
        project.mark_failed(id, &e)?;
        state.publish(Event::Run {
            project: params.project.clone(),
            run: Box::new(project.run_record(id)?),
        });
        return Err(ApiError::Internal(e));
    }
    Ok(record)
}

fn cancel_run(state: &AppState, _: &Principal, params: RunRef) -> ApiResult<RunRecord> {
    let project = state.open(&params.project)?;
    let was_queued = project.cancel_queued(params.id)?;
    let signalled = state.runner().cancel(&params.project, params.id) != Cancel::None;
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
        run: Box::new(record.clone()),
    });
    Ok(record)
}

fn cancel_operation(
    state: &AppState,
    principal: &Principal,
    params: OperationRef,
) -> ApiResult<PendingOperation> {
    operations::cancel_pending(state, principal, params.id)
}

fn pause_run(state: &AppState, _: &Principal, params: RunRef) -> ApiResult<RunRecord> {
    hold(state, params, true)
}

fn resume_run(state: &AppState, _: &Principal, params: RunRef) -> ApiResult<RunRecord> {
    hold(state, params, false)
}

/// Pause or resume a run held by a lane.
fn hold(state: &AppState, params: RunRef, paused: bool) -> ApiResult<RunRecord> {
    let project = state.open(&params.project)?;
    let record = project.run_record(params.id)?;
    if !state.runner().pause(&params.project, params.id, paused) {
        return Err(ApiError::Engine(accessmod_io::error::Error::invalid(
            format!(
                "run {} is {}: only a queued or running run is paused or resumed",
                params.id,
                record.status.name()
            ),
        )));
    }
    Ok(record)
}

fn retry_run(state: &AppState, _: &Principal, params: RunRef) -> ApiResult<RunRecord> {
    let project = state.open(&params.project)?;
    let id = project.retry_run(params.id)?;
    queue_analysis(state, &project, &params.project, id)
}

fn remove_run(state: &AppState, _: &Principal, params: RemoveRun) -> ApiResult<Value> {
    let project = state.open(&params.project)?;
    project.remove_run(params.id, params.outputs)?;
    state.publish(Event::Runs {
        project: params.project.clone(),
    });
    catalogue(state, &params.project);
    Ok(Value::Null)
}

fn catalogue(state: &AppState, project: &str) {
    state.publish(Event::Catalogue {
        project: project.to_owned(),
    });
}
