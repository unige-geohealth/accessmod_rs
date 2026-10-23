//! Map previews: raster tiles (cached per project), GeoJSON, values at a
//! point, bounds.

use std::path::PathBuf;

use accessmod_io::class::Kind;
use accessmod_io::preview::Resampling;
use accessmod_io::project::{DatasetRecord, Id, Project, Summary, resampling};
use accessmod_io::tiles::{self, Style};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Principal, Requirement, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Features sent at most by `GET …/features`.
const MAX_FEATURES: usize = 20_000;

/// Optional ramp bounds of a tile request.
#[derive(Debug, Deserialize)]
pub struct TileQuery {
    /// Value of the first colour (default: the dataset minimum).
    pub min: Option<f64>,
    /// Value of the last colour (default: the dataset maximum).
    pub max: Option<f64>,
}

fn dataset(project: &Project, id: Id, kind: Kind) -> ApiResult<DatasetRecord> {
    let record = project
        .dataset(id)
        .map_err(|_| ApiError::NotFound(format!("no dataset {id}")))?;
    if record.class.kind() != kind {
        return Err(ApiError::BadRequest(format!(
            "dataset {id} is a {:?} dataset",
            record.class.kind()
        )));
    }
    Ok(record)
}

/// Style of a raster dataset: classes, or a ramp over its range.
fn style(record: &DatasetRecord, query: &TileQuery) -> Style {
    if resampling(record.class) == Resampling::Nearest {
        return Style::Classes;
    }
    let (min, max) = match &record.summary {
        Summary::Raster(preview) => (
            preview.stats.min.unwrap_or(0.0),
            preview.stats.max.unwrap_or(1.0),
        ),
        _ => (0.0, 1.0),
    };
    Style::Ramp {
        min: query.min.unwrap_or(min),
        max: query.max.unwrap_or(max),
    }
}

fn parse_coordinate(text: &str) -> ApiResult<u32> {
    text.trim_end_matches(".png")
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("invalid tile coordinate {text:?}")))
}

/// `GET /api/projects/{project}/datasets/{id}/tiles/{z}/{x}/{y}` (`y` may
/// end with `.png`): a PNG tile in Web Mercator.
pub async fn tile(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id, z, x, y)): Path<(String, Id, String, String, String)>,
    Query(query): Query<TileQuery>,
) -> ApiResult<Response> {
    principal.authorize(Requirement::Project(Role::Viewer), Some(&slug))?;
    let (z, x, y) = (
        parse_coordinate(&z)?,
        parse_coordinate(&x)?,
        parse_coordinate(&y)?,
    );
    for bound in [query.min, query.max].into_iter().flatten() {
        if !bound.is_finite() {
            return Err(ApiError::BadRequest("ramp bounds must be finite".into()));
        }
    }
    // Only the default style is cached: custom ramps cannot fill the disk.
    let custom = query.min.is_some() || query.max.is_some();
    let png = state
        .blocking(move |state| {
            let project = state.open(&slug)?;
            let record = dataset(&project, id, Kind::Raster)?;
            let style = style(&record, &query);
            // Keyed by content: a dataset never changes under its sha256.
            let sha = record.sha256.get(..16).unwrap_or(&record.sha256);
            let cached: PathBuf = project
                .cache_dir()
                .join("tiles")
                .join(format!("{id}-{sha}"))
                .join(z.to_string())
                .join(x.to_string())
                .join(format!("{y}.png"));
            if !custom && let Ok(bytes) = std::fs::read(&cached) {
                return Ok(bytes);
            }
            let path = project.resolve(std::path::Path::new(&record.file))?;
            let tile = tiles::raster_tile(&path, z, x, y, style)?;
            // Empty tiles (outside the data) are cheap to render: not cached.
            // The cache is disposable: a failed write only costs a re-render.
            if !custom
                && !tile.empty
                && let Some(dir) = cached.parent()
                && std::fs::create_dir_all(dir).is_ok()
            {
                let partial = cached.with_extension(format!("png.{}", std::process::id()));
                if std::fs::write(&partial, &tile.png).is_ok() {
                    let _ = std::fs::rename(&partial, &cached);
                }
            }
            Ok(tile.png)
        })
        .await?;

    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        png,
    )
        .into_response())
}

/// Query of `GET …/features`.
#[derive(Debug, Deserialize)]
pub struct FeaturesQuery {
    /// Features at most (capped).
    pub limit: Option<usize>,
}

/// `GET /api/projects/{project}/datasets/{id}/features`: GeoJSON in WGS 84.
pub async fn features(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
    Query(query): Query<FeaturesQuery>,
) -> ApiResult<Json<Value>> {
    principal.authorize(Requirement::Project(Role::Viewer), Some(&slug))?;
    let limit = query.limit.unwrap_or(MAX_FEATURES).min(MAX_FEATURES);
    state
        .blocking(move |state| {
            let project = state.open(&slug)?;
            let record = dataset(&project, id, Kind::Vector)?;
            let path = project.resolve(std::path::Path::new(&record.file))?;
            Ok(tiles::geojson(&path, limit)?)
        })
        .await
        .map(Json)
}

/// Query of `GET …/value`.
#[derive(Debug, Deserialize)]
pub struct PointQuery {
    /// Longitude, degrees.
    pub lon: f64,
    /// Latitude, degrees.
    pub lat: f64,
}

/// `GET /api/projects/{project}/datasets/{id}/value?lon=&lat=`.
pub async fn value(
    State(state): State<AppState>,
    principal: Principal,
    Path((slug, id)): Path<(String, Id)>,
    Query(point): Query<PointQuery>,
) -> ApiResult<Json<Value>> {
    principal.authorize(Requirement::Project(Role::Viewer), Some(&slug))?;
    state
        .blocking(move |state| {
            let project = state.open(&slug)?;
            let record = dataset(&project, id, Kind::Raster)?;
            let path = project.resolve(std::path::Path::new(&record.file))?;
            let value = tiles::value_at(&path, point.lon, point.lat)?;
            Ok(json!({"value": value}))
        })
        .await
        .map(Json)
}

/// `GET /api/projects/{project}/bounds`: the project grid in WGS 84,
/// `[west, south, east, north]`.
pub async fn bounds(
    State(state): State<AppState>,
    principal: Principal,
    Path(slug): Path<String>,
) -> ApiResult<Json<[f64; 4]>> {
    principal.authorize(Requirement::Project(Role::Viewer), Some(&slug))?;
    state
        .blocking(move |state| {
            let project = state.open(&slug)?;
            let dem = project.dataset(project.info()?.dem)?;
            let path = project.resolve(std::path::Path::new(&dem.file))?;
            Ok(tiles::bounds_wgs84(&path)?)
        })
        .await
        .map(Json)
}
