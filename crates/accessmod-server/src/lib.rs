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
pub mod routes;
pub mod runner;
pub mod state;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Router;
use axum::routing::{get, post};
use tower_http::services::{ServeDir, ServeFile};

pub use state::{AppState, Config};

/// The API routes, and the front end from `web_dir` when given.
pub fn router(state: AppState, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/session", get(routes::session))
        .route("/api/analyses", get(routes::analyses))
        .route("/api/classes", get(routes::classes))
        .route("/api/commands", get(routes::command_list))
        .route("/api/commands/{id}", post(routes::command))
        .route("/api/events", get(events::stream))
        .route("/api/projects", get(routes::projects))
        .route("/api/projects/{project}", get(routes::project))
        .route("/api/projects/{project}/datasets", get(routes::datasets))
        .route(
            "/api/projects/{project}/datasets/{id}",
            get(routes::dataset),
        )
        .route("/api/projects/{project}/jobs", get(routes::jobs))
        .route("/api/projects/{project}/jobs/{id}", get(routes::job))
        .route("/api/projects/{project}/runs", get(routes::runs))
        .route("/api/projects/{project}/runs/{id}", get(routes::run))
        .route(
            "/api/projects/{project}/analyses/{name}/validate",
            post(routes::validate),
        )
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

/// Serve on `address` until Ctrl-C.
pub async fn serve(
    state: AppState,
    address: SocketAddr,
    web_dir: Option<PathBuf>,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!("accessmod serving on http://{}", listener.local_addr()?);
    axum::serve(listener, router(state, web_dir))
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
) -> std::io::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(state, address, web_dir))
}
