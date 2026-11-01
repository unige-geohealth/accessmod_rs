//! Read endpoints (queries) and the command endpoint.

use accessmod_io::analysis::{Monitor, job_schemas};
use accessmod_io::class::{DataClass, Kind};
use accessmod_io::project::{DatasetRecord, Id, ProjectInfo, RunRecord, SavedJob};
use accessmod_io::rows::{self, Rows};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower::ServiceExt;
use tower_http::services::ServeFile;

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

/// Any other `/api/…` path: a `not_found` error document.
pub async fn no_route(uri: axum::http::Uri) -> ApiError {
    ApiError::NotFound(format!("no API route {}", uri.path()))
}

/// `GET /api/settings`.
pub async fn settings(
    _: Principal,
    State(state): State<AppState>,
) -> Json<crate::settings::Settings> {
    Json(state.settings())
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

/// Query of `GET …/rows`.
#[derive(Debug, Deserialize)]
pub struct RowsQuery {
    /// Rows to skip.
    pub offset: Option<u64>,
    /// Rows at most (capped at [`rows::MAX_PAGE`]).
    pub limit: Option<usize>,
    /// OGR SQL attribute filter, vector datasets only.
    #[serde(rename = "where")]
    pub filter: Option<String>,
    /// Raster dataset ids, comma separated, read at the point of each row
    /// of a vector dataset.
    pub sample: Option<String>,
}

/// `GET /api/projects/{project}/datasets/{id}/rows`: a page of the
/// attributes of a vector dataset or of the records of a table.
pub async fn rows(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
    Query(query): Query<RowsQuery>,
) -> ApiResult<Json<Rows>> {
    with_project(&state, principal, slug, Role::Viewer, move |_, p| {
        let record = p
            .dataset(id)
            .map_err(|_| ApiError::NotFound(format!("no dataset {id}")))?;
        let path = p.resolve(std::path::Path::new(&record.file))?;
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(rows::MAX_PAGE);
        let filter = query.filter.as_deref().filter(|f| !f.trim().is_empty());
        let mut sampled = Vec::new();
        for raster in query.sample.iter().flat_map(|s| s.split(',')) {
            let raster: Id = raster
                .trim()
                .parse()
                .map_err(|_| ApiError::BadRequest(format!("sample: {raster:?} is not an id")))?;
            let record = p
                .dataset(raster)
                .map_err(|_| ApiError::NotFound(format!("no dataset {raster}")))?;
            if record.class.kind() != Kind::Raster {
                return Err(ApiError::BadRequest(format!(
                    "sample: dataset {raster} is not a raster"
                )));
            }
            sampled.push(p.resolve(std::path::Path::new(&record.file))?);
        }
        let sampled: Vec<&std::path::Path> = sampled.iter().map(|p| p.as_path()).collect();
        match (record.class.kind(), filter, sampled.is_empty()) {
            (Kind::Vector, filter, _) => {
                Ok(rows::vector_rows(&path, filter, offset, limit, &sampled)?)
            }
            (Kind::Table, None, true) => Ok(rows::table_rows(&path, offset, limit)?),
            (Kind::Table, _, _) => Err(ApiError::BadRequest(
                "a filter or samples apply to vector datasets only".into(),
            )),
            (kind, _, _) => Err(ApiError::BadRequest(format!(
                "dataset {id} is a {kind:?} dataset, without rows"
            ))),
        }
    })
    .await
    .map(Json)
}

/// Query of `GET …/values`.
#[derive(Debug, Deserialize)]
pub struct ValuesQuery {
    /// Field whose distinct values are listed.
    pub field: String,
    /// Field giving each value its label.
    pub label: Option<String>,
}

/// `GET /api/projects/{project}/datasets/{id}/values`: distinct values of
/// a field of a vector dataset (at most [`rows::MAX_VALUES`]), e.g. road
/// classes and their labels.
pub async fn values(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
    Query(query): Query<ValuesQuery>,
) -> ApiResult<Json<rows::FieldValues>> {
    with_project(&state, principal, slug, Role::Viewer, move |_, p| {
        let record = p
            .dataset(id)
            .map_err(|_| ApiError::NotFound(format!("no dataset {id}")))?;
        if record.class.kind() != Kind::Vector {
            return Err(ApiError::BadRequest(format!(
                "dataset {id} is not a vector dataset"
            )));
        }
        let path = p.resolve(std::path::Path::new(&record.file))?;
        let label = query.label.as_deref().filter(|l| !l.is_empty());
        Ok(rows::field_values(&path, &query.field, label)?)
    })
    .await
    .map(Json)
}

/// Format of `GET …/download`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadFormat {
    /// The table as stored.
    #[default]
    Csv,
    /// An array of objects keyed by column.
    Json,
}

/// Query of `GET …/download`.
#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    /// File format.
    #[serde(default)]
    pub format: DownloadFormat,
}

/// File name of a download: class code and tags, e.g.
/// `tZonalStat_demo.csv`; only ASCII letters, digits, `-` and `_`.
fn download_name(record: &DatasetRecord, extension: &str) -> String {
    let stem: String = std::iter::once(record.class.code())
        .chain(record.tags.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("_")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{stem}.{extension}")
}

/// `GET /api/projects/{project}/datasets/{id}/download`: a table dataset as
/// CSV (streamed) or JSON.
pub async fn download(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
    Query(query): Query<DownloadQuery>,
    request: Request,
) -> ApiResult<Response> {
    let format = query.format;
    let (record, path, json) = with_project(&state, principal, slug, Role::Viewer, move |_, p| {
        let record = p
            .dataset(id)
            .map_err(|_| ApiError::NotFound(format!("no dataset {id}")))?;
        if record.class.kind() != Kind::Table {
            return Err(ApiError::BadRequest(format!("dataset {id} is not a table")));
        }
        let path = p.resolve(std::path::Path::new(&record.file))?;
        let json = match format {
            DownloadFormat::Csv => None,
            DownloadFormat::Json => Some(rows::table_json(&path)?),
        };
        Ok((record, path, json))
    })
    .await?;
    let (mut response, extension) = match json {
        Some(json) => (
            ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
            "json",
        ),
        None => {
            let served = ServeFile::new(&path)
                .oneshot(request)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            (served.into_response(), "csv")
        }
    };
    let disposition = format!(
        "attachment; filename=\"{}\"",
        download_name(&record, extension)
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
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

/// Runs listed by `GET /api/runs` at most.
pub const MAX_JOBS: usize = 1000;

/// A run with its project (`GET /api/runs`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct JobEntry {
    /// Project slug.
    pub project: String,
    /// Project name.
    pub name: String,
    /// The run.
    pub run: RunRecord,
}

/// Query of `GET /api/runs`.
#[derive(Debug, Deserialize)]
pub struct JobsQuery {
    /// Runs at most (default and cap [`MAX_JOBS`]).
    pub limit: Option<usize>,
}

/// `GET /api/runs`: the latest runs of every project the user may see,
/// newest first (runs share one queue).
pub async fn all_runs(
    State(state): State<AppState>,
    principal: Principal,
    Query(query): Query<JobsQuery>,
) -> ApiResult<Json<Vec<JobEntry>>> {
    let limit = query.limit.unwrap_or(MAX_JOBS).min(MAX_JOBS);
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
                let listed = state.open(&slug).and_then(|p| {
                    let name = p.info()?.name;
                    Ok(p.recent_runs(limit)?
                        .into_iter()
                        .map(|run| JobEntry {
                            project: slug.clone(),
                            name: name.clone(),
                            run,
                        })
                        .collect::<Vec<_>>())
                });
                match listed {
                    Ok(runs) => out.extend(runs),
                    Err(e) => tracing::warn!("project {slug}: {e}"),
                }
                // The newest `limit` so far: memory stays bounded whatever
                // the number of projects.
                out.sort_by(|a, b| {
                    b.run
                        .started
                        .cmp(&a.run.started)
                        .then_with(|| b.run.id.cmp(&a.run.id))
                });
                out.truncate(limit);
            }
            Ok(out)
        })
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
