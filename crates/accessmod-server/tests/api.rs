//! The HTTP API in process: queries, commands, a run with its events,
//! errors as JSON documents with the right status.

use std::path::Path;
use std::time::Duration;

use accessmod_io::fixtures::{
    EPSG, X0, Y0, geo_transform, grid, settings, write_features, write_raster,
};
use accessmod_server::events::Event;
use accessmod_server::{AppState, Config, router};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use gdal::vector::{FieldValue, OGRFieldType};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const COLS: usize = 6;
const ROWS: usize = 4;

fn sources(dir: &Path) {
    write_raster::<f32>(
        &dir.join("dem.tif"),
        COLS,
        ROWS,
        geo_transform(X0, Y0),
        vec![10.0; COLS * ROWS],
        Some(-9999.0),
    );
    write_raster(
        &dir.join("lc.tif"),
        COLS,
        ROWS,
        geo_transform(X0, Y0),
        vec![1u8; COLS * ROWS],
        Some(0.0),
    );
    let (x, y) = grid(COLS, ROWS).cell_centre(1, 1);
    write_features(
        &dir.join("hf.gpkg"),
        "facilities",
        EPSG,
        &[("hf_id", OGRFieldType::OFTInteger)],
        &[(
            &format!("POINT ({x} {y})"),
            vec![Some(FieldValue::IntegerValue(7))],
        )],
    );
}

struct Server {
    dir: tempfile::TempDir,
    state: AppState,
    app: Router,
}

fn server(local_paths: bool) -> Server {
    let dir = tempfile::tempdir().expect("tempdir");
    sources(dir.path());
    let state = AppState::new(Config {
        data_dir: dir.path().join("data"),
        settings: settings(64 << 20, 1),
        local_paths,
    })
    .expect("state");
    let app = router(state.clone(), None);
    Server { dir, state, app }
}

impl Server {
    fn path(&self, file: &str) -> String {
        self.dir.path().join(file).display().to_string()
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .expect("request");
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        (status, value)
    }

    async fn command(&self, id: &str, params: Value) -> (StatusCode, Value) {
        self.call("POST", &format!("/api/commands/{id}"), Some(params))
            .await
    }

    /// Create project "Demo" with land cover (merged) and facilities.
    async fn demo(&self) -> String {
        let (status, created) = self
            .command(
                "project.create",
                json!({"name": "Demo", "dem": self.path("dem.tif")}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let slug = created["slug"].as_str().expect("slug").to_owned();
        for (file, class, tags) in [
            ("lc.tif", "rLandCoverMerged", ["base"]),
            ("hf.gpkg", "vFacility", ["health"]),
        ] {
            let (status, body) = self
                .command(
                    "dataset.import",
                    json!({"project": slug, "path": self.path(file), "class": class, "tags": tags}),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        slug
    }
}

fn travel_time_job() -> Value {
    json!({
        "schema_version": 1,
        "dem": "datasets/1.tif",
        "land_cover": "datasets/2.tif",
        "facilities": {"path": "datasets/3.fgb", "id_field": "hf_id"},
        "scenario": [{"class": 1, "mode": "motorized", "speed_kmh": 10.0}],
        "method": "isotropic",
        "direction": "from_facilities",
        "neighbourhood": 8
    })
}

#[tokio::test]
async fn listings_describe_analyses_classes_and_commands() {
    let s = server(true);
    let (status, health) = s.call("GET", "/api/health", None).await;
    assert_eq!(
        (status, health["status"].clone()),
        (StatusCode::OK, json!("ok"))
    );
    let (_, analyses) = s.call("GET", "/api/analyses", None).await;
    let names: Vec<&str> = analyses
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|a| a["name"].as_str())
        .collect();
    assert!(
        names.contains(&"travel-time") && names.len() == 8,
        "{names:?}"
    );
    let (_, classes) = s.call("GET", "/api/classes", None).await;
    assert!(
        classes
            .as_array()
            .expect("array")
            .iter()
            .any(|c| c["class"] == "vFacility")
    );
    let (_, commands) = s.call("GET", "/api/commands", None).await;
    let commands = commands.as_array().expect("array");
    assert!(
        commands.iter().all(|c| c["params"]["type"] == "object"),
        "every command has a parameter schema"
    );
    let (_, session) = s.call("GET", "/api/session", None).await;
    assert_eq!(session["user"], "local");
}

#[tokio::test]
async fn project_import_validate_and_run_with_events() {
    let s = server(true);
    let slug = s.demo().await;
    let (_, projects) = s.call("GET", "/api/projects", None).await;
    assert_eq!(projects[0]["slug"], json!(slug));
    assert_eq!(projects[0]["info"]["grid"]["cols"], COLS);
    let (_, datasets) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets?class=vFacility"),
            None,
        )
        .await;
    assert_eq!(datasets.as_array().map(Vec::len), Some(1));

    let (status, valid) = s
        .call(
            "POST",
            &format!("/api/projects/{slug}/analyses/travel-time/validate"),
            Some(travel_time_job()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{valid}");
    assert_eq!(valid["status"], "valid");

    let mut events = s.state.subscribe();
    let (status, run) = s
        .command(
            "run.start",
            json!({"project": slug, "analysis": "travel-time", "document": travel_time_job()}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["status"], "queued");
    let id = run["id"].as_i64().expect("id");

    // Events: queued, progress stages, then the finished run.
    let mut stages = Vec::new();
    let finished = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Ok(Event::Progress { progress, .. }) => stages.push(progress.stage),
                Ok(Event::Run { run, .. }) if run.id == id && run.status.name() != "queued" => {
                    return run;
                }
                Ok(_) => {}
                Err(e) => panic!("events: {e}"),
            }
        }
    })
    .await
    .expect("the run finishes");
    assert_eq!(finished.status.name(), "done", "{:?}", finished.error);
    assert!(stages.iter().any(|s| s == "solve"), "{stages:?}");
    assert_eq!(finished.datasets.len(), 2);

    let (_, record) = s
        .call("GET", &format!("/api/projects/{slug}/runs/{id}"), None)
        .await;
    assert_eq!(record["status"], "done");
    let (_, outputs) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets?class=rTravelTime"),
            None,
        )
        .await;
    assert_eq!(
        outputs[0]["file"],
        format!("runs/{id}/travel_time_seconds.tif")
    );

    // Nothing left to cancel.
    let (status, body) = s
        .command("run.cancel", json!({"project": slug, "id": id}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[tokio::test]
async fn queued_run_can_be_cancelled() {
    let s = server(true);
    let slug = s.demo().await;
    // Recorded as queued but not handed to the worker: deterministic.
    let project = s.state.open(&slug).expect("open");
    let id = project
        .start_named("travel-time", &travel_time_job(), &[])
        .expect("queue");
    let (status, run) = s
        .command("run.cancel", json!({"project": slug, "id": id}))
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["status"], "cancelled");
}

#[tokio::test]
async fn errors_are_json_with_their_status() {
    let s = server(true);
    let slug = s.demo().await;
    let (status, body) = s.call("GET", "/api/projects/nope/datasets", None).await;
    assert_eq!(
        (status, body["kind"].clone()),
        (StatusCode::NOT_FOUND, json!("not_found"))
    );
    let (status, _) = s.call("GET", "/api/projects/..%2Fetc/datasets", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = s.command("no.such", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = s.command("dataset.tag", json!({"project": slug})).await;
    assert_eq!(
        (status, body["kind"].clone()),
        (StatusCode::BAD_REQUEST, json!("bad_request"))
    );

    let mut job = travel_time_job();
    job["facilities"]["id_field"] = json!("missing");
    let (status, body) = s
        .call(
            "POST",
            &format!("/api/projects/{slug}/analyses/travel-time/validate"),
            Some(job),
        )
        .await;
    assert_eq!(
        (status, body["kind"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("validation"))
    );
    assert!(body["issues"].to_string().contains("missing"), "{body}");

    let mut job = travel_time_job();
    job["land_cover"] = json!("../../lc.tif");
    let (status, body) = s
        .command(
            "run.start",
            json!({"project": slug, "analysis": "travel-time", "document": job}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["issues"].to_string().contains("leaves the project"),
        "{body}"
    );
}

#[tokio::test]
async fn a_country_server_refuses_local_paths() {
    let s = server(false);
    let (status, body) = s
        .command(
            "project.create",
            json!({"name": "Demo", "dem": s.path("dem.tif")}),
        )
        .await;
    assert_eq!(
        (status, body["kind"].clone()),
        (StatusCode::FORBIDDEN, json!("forbidden"))
    );
}

#[tokio::test]
async fn events_are_a_stream() {
    let s = server(true);
    let request = Request::builder()
        .uri("/api/events")
        .body(Body::empty())
        .expect("request");
    let response = s.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
}

#[tokio::test]
async fn map_previews_tiles_features_values_and_bounds() {
    let s = server(true);
    let slug = s.demo().await;
    let (status, bounds) = s
        .call("GET", &format!("/api/projects/{slug}/bounds"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{bounds}");
    let b: Vec<f64> = bounds
        .as_array()
        .expect("bounds")
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    let (lon, lat) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    let (x, y) = accessmod_io::tiles::tile_at(lon, lat, 12);

    let request = Request::builder()
        .uri(format!(
            "/api/projects/{slug}/datasets/2/tiles/12/{x}/{y}.png"
        ))
        .body(Body::empty())
        .expect("request");
    let response = s.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    let png = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert!(png.starts_with(b"\x89PNG"));
    let cached = s
        .dir
        .path()
        .join("data/projects")
        .join(&slug)
        .join("cache/tiles");
    assert!(cached.exists(), "the tile is cached");

    let (status, value) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets/2/value?lon={lon}&lat={lat}"),
            None,
        )
        .await;
    assert_eq!(
        (status, value["value"].clone()),
        (StatusCode::OK, json!(1.0))
    );
    let (status, features) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets/3/features"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(features["features"][0]["properties"]["hf_id"], 7);
    // A raster has no features; a vector no tiles.
    let (status, _) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets/2/features"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets/3/tiles/1/0/0"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
