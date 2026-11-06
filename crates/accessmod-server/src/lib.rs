//! `accessmod serve`: the JSON API, events and web front end over projects
//! and analyses (`docs/ui.md`).
//!
//! - Queries are `GET` routes; every change is a command of one registry
//!   (`POST /api/commands/{id}`), checked against the user's role.
//! - Runs execute one at a time on a worker; their progress and states are
//!   server-sent events (`GET /api/events`).
//! - Blocking work (SQLite, GDAL) runs off the async threads, and a panic
//!   becomes an error response: the server keeps running.

#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::todo,
        clippy::unimplemented
    )
)]

pub mod auth;
pub mod commands;
pub mod error;
pub mod events;
pub mod guard;
pub mod maps;
pub mod operations;
pub mod routes;
pub mod runner;
pub mod settings;
pub mod state;
pub mod uploads;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{any, get, post};
use tower_http::services::{ServeDir, ServeFile};

pub use guard::Guard;
pub use state::{AppState, Config, DEFAULT_UPLOAD_LIMIT};

/// The API routes, and the front end from `web_dir` when given.
pub fn router(state: AppState, web_dir: Option<PathBuf>) -> Router {
    // Uploads stream to disk and count their file bytes against the limit;
    // the body adds part headers and boundaries (1 MiB is far more than
    // `MAX_FILES` parts need). Other bodies keep axum's default limit.
    let upload_limit =
        usize::try_from(state.config().upload_limit.saturating_add(1 << 20)).unwrap_or(usize::MAX);
    let api = Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/session", get(routes::session))
        .route("/api/settings", get(routes::settings))
        .route("/api/analyses", get(routes::analyses))
        .route("/api/classes", get(routes::classes))
        .route("/api/commands", get(routes::command_list))
        .route("/api/commands/{id}", post(routes::command))
        .route("/api/events", get(events::stream))
        .route("/api/runs", get(routes::all_runs))
        .route("/api/queue", get(routes::queue))
        .route("/api/operations", get(routes::operations))
        .route("/api/basemap", get(maps::basemap))
        .route(
            "/api/uploads",
            post(uploads::upload).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route("/api/projects", get(routes::projects))
        .route("/api/projects/{project}", get(routes::project))
        .route("/api/projects/{project}/datasets", get(routes::datasets))
        .route(
            "/api/projects/{project}/datasets/{id}",
            get(routes::dataset),
        )
        .route("/api/projects/{project}/bounds", get(maps::bounds))
        .route(
            "/api/projects/{project}/datasets/{id}/tiles/{z}/{x}/{y}",
            get(maps::tile),
        )
        .route(
            "/api/projects/{project}/datasets/{id}/features",
            get(maps::features),
        )
        .route(
            "/api/projects/{project}/datasets/{id}/rows",
            get(routes::rows),
        )
        .route(
            "/api/projects/{project}/datasets/{id}/values",
            get(routes::values),
        )
        .route(
            "/api/projects/{project}/datasets/{id}/download",
            get(routes::download),
        )
        .route(
            "/api/projects/{project}/datasets/{id}/value",
            get(maps::value),
        )
        .route(
            "/api/projects/{project}/exports/{name}",
            get(routes::export),
        )
        .route("/api/projects/{project}/archives", get(routes::archives))
        .route(
            "/api/projects/{project}/archives/{name}",
            get(routes::archive),
        )
        .route("/api/projects/{project}/jobs", get(routes::jobs))
        .route("/api/projects/{project}/jobs/{id}", get(routes::job))
        .route("/api/projects/{project}/runs", get(routes::runs))
        .route("/api/projects/{project}/runs/{id}", get(routes::run))
        .route(
            "/api/projects/{project}/analyses/{name}/validate",
            post(routes::validate),
        )
        // An unknown API path is a JSON error, never the client's page.
        .route("/api/{*path}", any(routes::no_route))
        .with_state(state);
    match web_dir {
        // A single-page application: unknown paths serve `index.html`.
        Some(dir) => {
            let index = dir.join("index.html");
            api.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => api,
    }
}

/// Serve on `address` behind `guard` until Ctrl-C. `ready` receives the
/// bound address (port 0 picks a free port).
pub async fn serve(
    state: AppState,
    address: SocketAddr,
    web_dir: Option<PathBuf>,
    guard: Guard,
    ready: impl FnOnce(SocketAddr),
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    let bound = listener.local_addr()?;
    tracing::info!("accessmod serving on http://{bound}");
    ready(bound);
    axum::serve(listener, guard.wrap(router(state, web_dir)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

/// [`serve`] on a runtime of its own, for callers without one (the CLI).
pub fn serve_blocking(
    state: AppState,
    address: SocketAddr,
    web_dir: Option<PathBuf>,
    guard: Guard,
    ready: impl FnOnce(SocketAddr),
) -> std::io::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(state, address, web_dir, guard, ready))
}
