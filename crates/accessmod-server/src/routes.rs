//! Read endpoints (queries) and the command endpoint.

use accessmod_io::analysis::{Monitor, job_schemas};
use accessmod_io::class::DataClass;
use accessmod_io::project::{DatasetRecord, Id, ProjectInfo, RunRecord, SavedJob};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth::{Principal, Requirement, Role};
use crate::commands::{self, CommandInfo};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// A JSON request body; malformed JSON is a `bad_request` error document.
fn json_body(body: &[u8]) -> ApiResult<Value> {
    serde_json::from_slice(body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))
}

/// `GET /api/health`.
pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}

/// `GET /api/session`: who the server sees.
pub async fn session(principal: Principal) -> Json<Principal> {
    Json(principal)
}

/// An analysis and the JSON Schema of its job.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AnalysisInfo {
    /// Name, e.g. `travel-time`.
    pub name: &'static str,
    /// JSON Schema of its job document.
    pub schema: Value,
}

/// `GET /api/analyses`.
pub async fn analyses() -> Json<Vec<AnalysisInfo>> {
    Json(
        job_schemas()
            .into_iter()
            .map(|(name, schema)| AnalysisInfo { name, schema })
            .collect(),
    )
}

/// `GET /api/classes`: data classes, with kind, label and importability.
pub async fn classes() -> Json<Vec<Value>> {
    Json(
        DataClass::ALL
            .iter()
            .map(|c| {
                json!({
                    "class": c.code(),
                    "kind": c.kind(),
                    "label": c.label(),
                    "importable": c.importable(),
                })
            })
            .collect(),
    )
}

/// `GET /api/commands`.
pub async fn command_list() -> Json<Vec<CommandInfo>> {
    Json(commands::listing())
}

/// `POST /api/commands/{id}`: run a command with the body as parameters.
pub async fn command(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let params = json_body(&body)?;
    state
        .blocking(move |state| commands::run(&state, &principal, &id, params))
        .await
        .map(Json)
}

/// A project of the listing.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectEntry {
    /// Slug.
    pub slug: String,
    /// Name, grid and DEM.
    pub info: ProjectInfo,
}

/// `GET /api/projects`: projects the user may see.
pub async fn projects(
    State(state): State<AppState>,
    principal: Principal,
) -> ApiResult<Json<Vec<ProjectEntry>>> {
    state
        .blocking(move |state| {
            let mut out = Vec::new();
            for slug in state.project_slugs() {
                if principal
                    .authorize(Requirement::Project(Role::Viewer), Some(&slug))
                    .is_err()
                {
                    continue;
                }
                match state.open(&slug).and_then(|p| Ok(p.info()?)) {
                    Ok(info) => out.push(ProjectEntry { slug, info }),
                    Err(e) => tracing::warn!("project {slug}: {e}"),
                }
            }
            Ok(out)
        })
        .await
        .map(Json)
}

/// Open `slug` for `principal` with at least `role` and run `f` on it.
async fn with_project<T: Send + 'static>(
    state: &AppState,
    principal: Principal,
    slug: String,
    role: Role,
    f: impl FnOnce(&AppState, accessmod_io::project::Project) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    principal.authorize(Requirement::Project(role), Some(&slug))?;
    state
        .blocking(move |state| {
            let project = state.open(&slug)?;
            f(&state, project)
        })
        .await
}

/// `GET /api/projects/{project}`.
pub async fn project(
    State(state): State<AppState>,
    principal: Principal,
    Path(slug): Path<String>,
) -> ApiResult<Json<ProjectInfo>> {
    with_project(&state, principal, slug, Role::Viewer, |_, p| Ok(p.info()?))
        .await
        .map(Json)
}

/// Query of the dataset listing.
#[derive(Debug, Deserialize)]
pub struct DatasetQuery {
    /// Only this class (AccessMod 5 identifier).
    pub class: Option<String>,
}

/// `GET /api/projects/{project}/datasets`.
pub async fn datasets(
    State(state): State<AppState>,
    principal: Principal,
    Path(slug): Path<String>,
    Query(query): Query<DatasetQuery>,
) -> ApiResult<Json<Vec<DatasetRecord>>> {
    let class = query
        .class
        .map(|c| {
            DataClass::from_code(&c)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown class {c:?}")))
        })
        .transpose()?;
    with_project(&state, principal, slug, Role::Viewer, move |_, p| {
        Ok(p.datasets(class)?)
    })
    .await
    .map(Json)
}

/// `GET /api/projects/{project}/datasets/{id}`.
pub async fn dataset(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
) -> ApiResult<Json<DatasetRecord>> {
    with_project(&state, principal, slug, Role::Viewer, move |_, p| {
        p.dataset(id)
            .map_err(|_| ApiError::NotFound(format!("no dataset {id}")))
    })
    .await
    .map(Json)
}

/// `GET /api/projects/{project}/jobs`.
pub async fn jobs(
    State(state): State<AppState>,
    principal: Principal,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<SavedJob>>> {
    with_project(&state, principal, slug, Role::Viewer, |_, p| Ok(p.jobs()?))
        .await
        .map(Json)
}

/// `GET /api/projects/{project}/jobs/{id}`.
pub async fn job(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
) -> ApiResult<Json<SavedJob>> {
    with_project(&state, principal, slug, Role::Viewer, move |_, p| {
        p.job(id)
            .map_err(|_| ApiError::NotFound(format!("no saved job {id}")))
    })
    .await
    .map(Json)
}

/// `GET /api/projects/{project}/runs`.
pub async fn runs(
    State(state): State<AppState>,
    principal: Principal,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<RunRecord>>> {
    with_project(&state, principal, slug, Role::Viewer, |_, p| Ok(p.runs()?))
        .await
        .map(Json)
}

/// `GET /api/projects/{project}/runs/{id}`.
pub async fn run(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
) -> ApiResult<Json<RunRecord>> {
    with_project(&state, principal, slug, Role::Viewer, move |_, p| {
        p.run_record(id)
            .map_err(|_| ApiError::NotFound(format!("no run {id}")))
    })
    .await
    .map(Json)
}

/// `POST /api/projects/{project}/analyses/{name}/validate`: check a job
/// document against the inputs, without computing.
pub async fn validate(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, name)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let document = json_body(&body)?;
    with_project(&state, principal, slug, Role::Viewer, move |state, p| {
        Ok(p.validate_named(&name, &document, &state.config().settings, Monitor::QUIET)?)
    })
    .await
    .map(Json)
}
