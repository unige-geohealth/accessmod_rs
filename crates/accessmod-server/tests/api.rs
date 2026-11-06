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
    server_with(local_paths, accessmod_server::DEFAULT_UPLOAD_LIMIT)
}

fn server_with(local_paths: bool, upload_limit: u64) -> Server {
    let dir = tempfile::tempdir().expect("tempdir");
    sources(dir.path());
    let state = state_on(dir.path(), local_paths, upload_limit, false);
    let app = router(state.clone(), None);
    Server { dir, state, app }
}

/// A server on the data directory of `dir`, building the demo project in
/// a new one when `demo`.
fn state_on(dir: &Path, local_paths: bool, upload_limit: u64, demo: bool) -> AppState {
    AppState::new(Config {
        data_dir: dir.join("data"),
        settings: settings(64 << 20, 1),
        local_paths,
        upload_limit,
        demo,
        maptiler_key: Some("test-key".into()),
    })
    .expect("state")
}

impl Server {
    /// A new server on the same data directory (a restart).
    fn restart(self, demo: bool) -> Self {
        let state = state_on(
            self.dir.path(),
            true,
            accessmod_server::DEFAULT_UPLOAD_LIMIT,
            demo,
        );
        let app = router(state.clone(), None);
        Self {
            dir: self.dir,
            state,
            app,
        }
    }
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

    /// `POST /api/uploads` with one part per `(file name, bytes)`.
    async fn upload(&self, files: &[(&str, Vec<u8>)]) -> (StatusCode, Value) {
        const BOUNDARY: &str = "accessmod-test-boundary";
        let mut body = Vec::new();
        for (name, bytes) in files {
            body.extend_from_slice(
                format!(
                    "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        let request = Request::builder()
            .method("POST")
            .uri("/api/uploads")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from(body))
            .expect("request");
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, value)
    }

    /// Upload the source files `names` (in the test directory).
    async fn upload_files(&self, names: &[&str]) -> String {
        let files: Vec<(&str, Vec<u8>)> = names
            .iter()
            .map(|n| (*n, std::fs::read(self.dir.path().join(n)).expect("read")))
            .collect();
        let (status, body) = self.upload(&files).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["id"].as_str().expect("id").to_owned()
    }

    fn uploads(&self) -> usize {
        std::fs::read_dir(self.dir.path().join("data/uploads"))
            .map(|d| d.count())
            .unwrap_or(0)
    }

    /// Create project "Demo" with land cover (merged) and facilities.
    async fn demo(&self) -> String {
        let (status, created) = self
            .command(
                "project.create",
                json!({"name": "Demo", "dem": {"path": self.path("dem.tif")}}),
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
                    json!({"project": slug, "source": {"path": self.path(file)}, "class": class, "tags": tags}),
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
    let mut states = Vec::new();
    let finished = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Ok(Event::Progress { progress, .. }) => stages.push(progress.stage),
                Ok(Event::Run { run, .. }) if run.id == id => {
                    states.push(run.status.name());
                    if !["queued", "running"].contains(&run.status.name()) {
                        return run;
                    }
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
    assert_eq!(states, ["queued", "running", "done"], "events in order");
    assert_eq!(finished.datasets.len(), 2);

    let (_, record) = s
        .call("GET", &format!("/api/projects/{slug}/runs/{id}"), None)
        .await;
    assert_eq!(record["status"], "done");
    // Every project's runs, newest first, with the project.
    let (status, jobs) = s.call("GET", "/api/runs?limit=5", None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    assert_eq!(jobs[0]["project"], json!(slug));
    assert_eq!(jobs[0]["name"], "Demo");
    assert_eq!(jobs[0]["run"]["id"], json!(id));
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
        .start_named("travel-time", &travel_time_job(), &[], false)
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
            json!({"name": "Demo", "dem": {"path": s.path("dem.tif")}}),
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

#[tokio::test]
async fn rows_page_vector_attributes_and_tables() {
    let s = server(true);
    let slug = s.demo().await;
    std::fs::write(
        s.dir.path().join("scenario.csv"),
        "class,label,speed,mode\n1,forest,5,WALKING\n2,road,60,MOTORIZED\n",
    )
    .expect("write");
    let (status, body) = s
        .command(
            "dataset.import",
            json!({"project": slug, "source": {"path": s.path("scenario.csv")}, "class": "tScenario", "tags": ["base"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = |query: &str| format!("/api/projects/{slug}/datasets/{query}");

    let (status, page) = s.call("GET", &rows("3/rows"), None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["columns"], json!(["hf_id"]));
    assert_eq!(page["rows"], json!([[7]]));
    assert_eq!(page["total"], 1);
    // Land cover at the facility: class 1.
    let (status, page) = s.call("GET", &rows("3/rows?sample=2,2"), None).await;
    assert_eq!(
        (status, page["samples"].clone()),
        (StatusCode::OK, json!([[1.0, 1.0]]))
    );
    let (status, page) = s
        .call("GET", &rows("3/rows?where=hf_id%20%3E%207"), None)
        .await;
    assert_eq!((status, page["total"].clone()), (StatusCode::OK, json!(0)));

    let (status, page) = s.call("GET", &rows("4/rows?offset=1&limit=5"), None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["rows"], json!([["2", "road", "60", "MOTORIZED"]]));
    assert_eq!(
        (page["offset"].clone(), page["total"].clone()),
        (json!(1), json!(2))
    );

    // Hostile or misplaced queries are client errors, never a crash.
    for (query, expected) in [
        (
            "3/rows?where=hf_id%20%3C%3C%3C",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "3/rows?where=1%3D1%3B%20DROP%20TABLE%20x",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("3/rows?offset=-1", StatusCode::BAD_REQUEST),
        ("3/rows?limit=abc", StatusCode::BAD_REQUEST),
        ("4/rows?where=class%3D1", StatusCode::BAD_REQUEST),
        ("2/rows", StatusCode::BAD_REQUEST),
        ("3/rows?sample=3", StatusCode::BAD_REQUEST),
        ("3/rows?sample=x", StatusCode::BAD_REQUEST),
        ("3/rows?sample=99", StatusCode::NOT_FOUND),
        ("3/rows?sample=2,2,2,2,2", StatusCode::UNPROCESSABLE_ENTITY),
        ("4/rows?sample=2", StatusCode::BAD_REQUEST),
        ("99/rows", StatusCode::NOT_FOUND),
    ] {
        let (status, body) = s.call("GET", &rows(query), None).await;
        assert_eq!(status, expected, "{query}: {body}");
    }
    let (status, page) = s
        .call(
            "GET",
            &rows("3/rows?offset=18446744073709551615&limit=99999999"),
            None,
        )
        .await;
    assert_eq!((status, page["rows"].clone()), (StatusCode::OK, json!([])));
}

#[tokio::test]
async fn tables_download_as_csv_or_json() {
    let s = server(true);
    let slug = s.demo().await;
    let csv = "class,label,speed,mode\n1,\"forest, dense\",5,WALKING\n";
    std::fs::write(s.dir.path().join("scenario.csv"), csv).expect("write");
    let (status, body) = s
        .command(
            "dataset.import",
            json!({"project": slug, "source": {"path": s.path("scenario.csv")}, "class": "tScenario", "tags": ["base", "dry season"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let get = |query: &str| {
        Request::builder()
            .uri(format!("/api/projects/{slug}/datasets/{query}"))
            .body(Body::empty())
            .expect("request")
    };
    let response = s
        .app
        .clone()
        .oneshot(get("4/download"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"tScenario_base_dry_season.csv\""
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let stored = String::from_utf8_lossy(&bytes);
    assert!(stored.contains("forest, dense"), "{stored}");

    let (status, json) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets/4/download?format=json"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json[0]["label"], "forest, dense");

    for (query, expected) in [
        ("2/download", StatusCode::BAD_REQUEST),
        ("3/download?format=json", StatusCode::BAD_REQUEST),
        ("4/download?format=xml", StatusCode::BAD_REQUEST),
        ("99/download", StatusCode::NOT_FOUND),
    ] {
        let (status, body) = s
            .call(
                "GET",
                &format!("/api/projects/{slug}/datasets/{query}"),
                None,
            )
            .await;
        assert_eq!(status, expected, "{query}: {body}");
    }
}

#[tokio::test]
async fn unknown_api_paths_are_json_errors_even_with_a_client() {
    let s = server(true);
    let web = tempfile::tempdir().expect("temp dir");
    std::fs::write(web.path().join("index.html"), "<!doctype html>").expect("write");
    let app = router(s.state.clone(), Some(web.path().to_path_buf()));
    for uri in ["/api/nope", "/api/projects/demo/datasets/1/nope"] {
        let request = Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = app.clone().oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let body: Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body["kind"], "not_found", "{uri}");
    }
    let request = Request::builder()
        .uri("/some/page")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "client routes still serve the page"
    );
}

#[tokio::test]
async fn malformed_bodies_get_json_errors() {
    let s = server(true);
    let request = Request::builder()
        .method("POST")
        .uri("/api/commands/project.create")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .expect("request");
    let response = s.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON error document");
    assert_eq!(body["kind"], "bad_request");
}

/// `hf.gpkg` as a shapefile: `hf.shp`, `.shx`, `.dbf`, `.prj`.
fn shapefile(dir: &Path) -> Vec<&'static str> {
    let source = gdal::Dataset::open(dir.join("hf.gpkg")).expect("open");
    let driver = gdal::DriverManager::get_driver_by_name("ESRI Shapefile").expect("driver");
    source
        .create_copy(
            &driver,
            dir.join("hf.shp"),
            &gdal::cpl::CslStringList::new(),
        )
        .expect("copy");
    vec!["hf.shp", "hf.shx", "hf.dbf", "hf.prj"]
}

#[tokio::test]
async fn uploads_create_projects_and_import_datasets() {
    // A country server: no local path, only uploads.
    let s = server(false);
    let dem = s.upload_files(&["dem.tif"]).await;
    let (status, created) = s
        .command(
            "project.create",
            json!({"name": "Uploaded", "dem": {"upload": {"id": dem}}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let slug = created["slug"].as_str().expect("slug").to_owned();
    assert_eq!(created["info"]["grid"]["cols"], COLS);
    assert_eq!(s.uploads(), 0, "a consumed upload is removed");

    // A VRT, whatever its name, would read any file of the server.
    let vrt = (COLS, ROWS);
    let evil = format!(
        r#"<VRTDataset rasterXSize="{}" rasterYSize="{}"><VRTRasterBand dataType="Float32" band="1"><SimpleSource><SourceFilename relativeToVRT="0">{}</SourceFilename><SourceBand>1</SourceBand></SimpleSource></VRTRasterBand></VRTDataset>"#,
        vrt.0,
        vrt.1,
        s.path("dem.tif")
    );
    let (status, up) = s.upload(&[("dem.tif", evil.into_bytes())]).await;
    assert_eq!(status, StatusCode::OK, "{up}");
    let (status, body) = s
        .command(
            "project.create",
            json!({"name": "Evil", "dem": {"upload": {"id": up["id"]}}}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["issues"].to_string().contains("not a supported file"),
        "{body}"
    );

    let files = shapefile(s.dir.path());
    let hf = s.upload_files(&files).await;
    let (status, record) = s
        .command(
            "dataset.import",
            json!({"project": slug, "source": {"upload": {"id": hf}}, "class": "vFacility", "tags": ["health"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{record}");
    assert_eq!(record["class"], "vFacility");

    // Several main files: the command names one.
    let both = s.upload_files(&["lc.tif", "dem.tif"]).await;
    let (status, body) = s
        .command(
            "dataset.import",
            json!({"project": slug, "source": {"upload": {"id": both}}, "class": "rLandCoverMerged", "tags": ["base"]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = s
        .command(
            "dataset.import",
            json!({"project": slug, "source": {"upload": {"id": both, "file": "lc.tif"}}, "class": "rLandCoverMerged", "tags": ["base"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(s.uploads(), 1, "only the refused VRT is left, for a retry");
}

#[tokio::test]
async fn uploads_are_confined_and_bounded() {
    let s = server_with(false, 1024);
    for name in ["../x.tif", "a/b.tif", ".owner", ""] {
        let (status, body) = s.upload(&[(name, b"x".to_vec())]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?}: {body}");
    }
    let (status, body) = s
        .upload(&[("a.tif", b"x".to_vec()), ("a.tif", b"y".to_vec())])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = s.upload(&[("big.tif", vec![0; 4096])]).await;
    assert!(status.is_client_error(), "{status}: {body}");
    assert_eq!(s.uploads(), 0, "failed uploads leave nothing");
    // The limit counts file bytes, not the multipart envelope.
    let (status, body) = s.upload(&[("exact.tif", vec![0; 1024])]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = s.upload(&[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    for id in ["0", "../projects", "zz", ""] {
        let (status, body) = s
            .command(
                "project.create",
                json!({"name": "X", "dem": {"upload": {"id": id}}}),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id:?}: {body}");
    }
    // Another user's upload is not found.
    let (status, up) = s.upload(&[("a.tif", b"x".to_vec())]).await;
    assert_eq!(status, StatusCode::OK, "{up}");
    let id = up["id"].as_str().expect("id");
    std::fs::write(
        s.dir.path().join("data/uploads").join(id).join(".owner"),
        "someone",
    )
    .expect("owner");
    let (status, body) = s
        .command(
            "project.create",
            json!({"name": "X", "dem": {"upload": {"id": id}}}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // A file outside the upload cannot be named.
    std::fs::write(
        s.dir.path().join("data/uploads").join(id).join(".owner"),
        "local",
    )
    .expect("owner");
    let (status, body) = s
        .command(
            "project.create",
            json!({"name": "X", "dem": {"upload": {"id": id, "file": "../../dem.tif"}}}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn projects_can_be_removed_when_idle() {
    let s = server(true);
    let slug = s.demo().await;
    let mut events = s.state.subscribe();
    let (status, run) = s
        .command(
            "run.start",
            json!({"project": slug, "analysis": "travel-time", "document": travel_time_job()}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    // Wait for the run to end: removal is refused while it is queued or running.
    let (status, _) = s.command("project.remove", json!({"project": slug})).await;
    if status != StatusCode::OK {
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if let Ok(Event::Run { run, .. }) = events.recv().await
                    && !matches!(run.status.name(), "queued" | "running")
                {
                    break;
                }
            }
        })
        .await
        .expect("run ends");
        let (status, body) = s.command("project.remove", json!({"project": slug})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, _) = s.call("GET", &format!("/api/projects/{slug}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = s.call("GET", "/api/projects", None).await;
    assert_eq!(list, json!([]));
    let (status, _) = s.command("project.remove", json!({"project": slug})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = s
        .command("project.remove", json!({"project": "../data"}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

fn slugs(list: &Value) -> Vec<String> {
    list.as_array()
        .expect("array")
        .iter()
        .filter_map(|p| p["slug"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn the_demo_project_is_built_once_and_reset_on_demand() {
    let dir = tempfile::tempdir().expect("tempdir");
    sources(dir.path());
    let state = state_on(
        dir.path(),
        true,
        accessmod_server::DEFAULT_UPLOAD_LIMIT,
        true,
    );
    let app = router(state.clone(), None);
    let s = Server { dir, state, app };
    let (_, list) = s.call("GET", "/api/projects", None).await;
    assert_eq!(slugs(&list), ["demo"]);
    assert_eq!(list[0]["info"]["demo"], json!(true));
    let (_, datasets) = s.call("GET", "/api/projects/demo/datasets", None).await;
    assert_eq!(datasets.as_array().map(Vec::len), Some(11));

    // Removed, it stays removed across restarts.
    let (status, body) = s
        .command("project.remove", json!({"project": "demo"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let s = s.restart(true);
    let (_, list) = s.call("GET", "/api/projects", None).await;
    assert_eq!(list, json!([]));

    // A user project named Demo does not take the slug of the demo.
    let (status, created) = s
        .command(
            "project.create",
            json!({"name": "Demo", "dem": {"path": s.path("dem.tif")}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["slug"], "demo-2");
    assert_eq!(created["info"]["demo"], json!(false));

    // Reset builds it again, twice in a row too.
    for _ in 0..2 {
        let (status, body) = s.command("demo.reset", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["slug"], "demo");
        assert_eq!(body["info"]["demo"], json!(true));
    }
    let (_, list) = s.call("GET", "/api/projects", None).await;
    assert_eq!(slugs(&list), ["demo", "demo-2"]);
    // Built aside then moved in place: nothing else is left.
    let mut entries: Vec<String> = std::fs::read_dir(s.dir.path().join("data/projects"))
        .expect("projects")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, ["demo", "demo-2"]);
    let (status, _) = s.command("demo.reset", json!({"force": true})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A project in the place of the demo that is not the demo is kept.
    let (status, _) = s
        .command("project.remove", json!({"project": "demo"}))
        .await;
    assert_eq!(status, StatusCode::OK);
    accessmod_io::project::Project::create(
        &s.dir.path().join("data/projects/demo"),
        "Mine",
        "",
        &s.dir.path().join("dem.tif"),
        &settings(64 << 20, 1),
        &|| false,
    )
    .expect("create");
    let (status, body) = s.command("demo.reset", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (_, project) = s.call("GET", "/api/projects/demo", None).await;
    assert_eq!(project["name"], "Mine");
}

#[tokio::test]
async fn the_default_project_is_a_setting() {
    let s = server(true);
    let slug = s.demo().await;
    let (status, current) = s.call("GET", "/api/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current, json!({"default_project": null}));
    let (status, body) = s
        .command("settings.default_project", json!({"project": slug}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["default_project"], json!(slug));
    // Kept across restarts.
    let s = s.restart(false);
    let (_, current) = s.call("GET", "/api/settings", None).await;
    assert_eq!(current["default_project"], json!(slug));
    for (params, expected) in [
        (json!({"project": "missing"}), StatusCode::NOT_FOUND),
        (json!({"project": "../data"}), StatusCode::NOT_FOUND),
        (json!({"project": 3}), StatusCode::BAD_REQUEST),
        (
            json!({"project": slug, "other": 1}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, body) = s.command("settings.default_project", params).await;
        assert_eq!(status, expected, "{body}");
    }
    let (_, current) = s.call("GET", "/api/settings", None).await;
    assert_eq!(current["default_project"], json!(slug));
    // Cleared by null, or when the project is removed.
    let (status, body) = s
        .command("settings.default_project", json!({"project": null}))
        .await;
    assert_eq!(
        (status, body["default_project"].clone()),
        (StatusCode::OK, Value::Null)
    );
    let (status, _) = s
        .command("settings.default_project", json!({"project": slug}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = s.command("project.remove", json!({"project": slug})).await;
    assert_eq!(status, StatusCode::OK);
    let (_, current) = s.call("GET", "/api/settings", None).await;
    assert_eq!(current["default_project"], Value::Null);
}

#[tokio::test]
async fn guard_needs_a_loopback_host_and_the_session_cookie() {
    let s = server(true);
    let app = accessmod_server::Guard {
        loopback_host: true,
        token: Some("0123456789abcdef".into()),
    }
    .wrap(s.app.clone());
    let send = |uri: &str, host: &str, cookie: Option<&str>| {
        let mut request = Request::builder().uri(uri).header("host", host);
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        app.clone()
            .oneshot(request.body(Body::empty()).expect("request"))
    };
    let health = "/api/health";
    let local = "127.0.0.1:5099";
    // DNS rebinding: a foreign name resolving to 127.0.0.1.
    let r = send(health, "evil.example:5099", None)
        .await
        .expect("response");
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // No session yet, or a wrong token.
    let r = send(health, local, None).await.expect("response");
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = send("/api/session/open?token=wrong", local, None)
        .await
        .expect("response");
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // The window opens the session, then uses its cookie.
    let r = send("/api/session/open?token=0123456789abcdef", local, None)
        .await
        .expect("response");
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let cookie = r.headers()["set-cookie"].to_str().expect("cookie");
    assert!(cookie.contains("SameSite=Strict") && cookie.contains("HttpOnly"));
    let pair = cookie.split(';').next().expect("pair");
    let r = send(health, "localhost:5099", Some(pair))
        .await
        .expect("response");
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn values_list_the_classes_of_a_field_with_labels() {
    let s = server(true);
    let slug = s.demo().await;
    let (x, y) = grid(COLS, ROWS).cell_centre(1, 1);
    let line = format!("LINESTRING ({x} {y}, {} {y})", x + 10.0);
    let road = |class: i32, label: &str| {
        (
            line.as_str(),
            vec![
                Some(FieldValue::IntegerValue(class)),
                Some(FieldValue::StringValue(label.into())),
            ],
        )
    };
    write_features(
        &s.dir.path().join("roads.gpkg"),
        "roads",
        EPSG,
        &[
            ("class_n", OGRFieldType::OFTInteger),
            ("label", OGRFieldType::OFTString),
        ],
        &[road(3, "Tertiary"), road(1, "Main road"), road(3, "Track")],
    );
    let (status, body) = s
        .command(
            "dataset.import",
            json!({"project": slug, "source": {"path": s.path("roads.gpkg")}, "class": "vRoad", "tags": ["roads"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let roads = body["id"].clone();
    let url = |query: &str| format!("/api/projects/{slug}/datasets/{roads}/values?{query}");
    let (status, body) = s.call("GET", &url("field=class_n&label=label"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The label of the first feature of each class.
    assert_eq!(
        body,
        json!({"complete": true, "values": [
            {"value": 1, "label": "Main road", "several_labels": false, "count": 1},
            {"value": 3, "label": "Tertiary", "several_labels": true, "count": 2},
        ]})
    );
    let (status, body) = s.call("GET", &url("field=nope"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[tokio::test]
async fn relocator_flow_through_the_api() {
    let s = server(true);
    let slug = s.demo().await;
    let features = |id: i64| format!("/api/projects/{slug}/datasets/{id}/features?sample=2");
    // The facility (dataset 3) sampled on the land cover (dataset 2).
    let (status, before) = s.call("GET", &features(3), None).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let feature = before["features"][0].clone();
    assert_eq!(feature["sample"], json!(1.0));
    let fid = feature["id"].clone();
    let [lon, lat] = [0, 1].map(|i| {
        feature["geometry"]["coordinates"][i]
            .as_f64()
            .expect("coordinate")
    });

    // Moved off the grid: no value, the facility is to inspect.
    let relocate = |id: i64, lon: f64, tags: &str| json!({"project": slug, "id": id, "moves": [{"fid": fid, "lon": lon, "lat": lat}], "tags": [tags]});
    let (status, off) = s
        .command("dataset.relocate", relocate(3, lon + 1.0, "off"))
        .await;
    assert_eq!(status, StatusCode::OK, "{off}");
    let off_id = off["id"].as_i64().expect("id");
    let (_, moved) = s.call("GET", &features(off_id), None).await;
    assert_eq!(moved["features"][0]["sample"], Value::Null);
    assert_eq!(moved["features"][0]["properties"], feature["properties"]);

    // Moved back on a cell with a value.
    let (status, back) = s
        .command("dataset.relocate", relocate(off_id, lon, "back"))
        .await;
    assert_eq!(status, StatusCode::OK, "{back}");
    let (_, again) = s
        .call("GET", &features(back["id"].as_i64().expect("id")), None)
        .await;
    assert_eq!(again["features"][0]["sample"], json!(1.0));
    // The source is unchanged.
    let (_, source) = s.call("GET", &features(3), None).await;
    assert_eq!(source, before);
    // A vector is not a sample raster.
    let (status, _) = s
        .call(
            "GET",
            &format!("/api/projects/{slug}/datasets/3/features?sample=3"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The basemap key of the server.
    let (_, basemap) = s.call("GET", "/api/basemap", None).await;
    assert_eq!(basemap, json!({"maptiler_key": "test-key"}));
}

#[tokio::test]
async fn projects_export_to_a_file_and_import_from_it() {
    let s = server(true);
    let slug = s.demo().await;
    let (status, exported) = s
        .command("project.export", json!({"project": slug, "results": false}))
        .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    let name = exported["name"].as_str().expect("name").to_owned();
    assert!(name.ends_with(".am6p"), "{name}");
    let response = s
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/projects/{slug}/exports/{name}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-disposition"],
        format!("attachment; filename=\"{name}\"").as_str()
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(
        bytes.len() as u64,
        exported["bytes"].as_u64().expect("bytes")
    );
    std::fs::write(s.dir.path().join("copy.am6p"), &bytes).expect("write");

    let (status, created) = s
        .command(
            "project.import",
            json!({"name": "Copy", "package": {"path": s.path("copy.am6p")}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let copy = created["slug"].as_str().expect("slug");
    assert_ne!(copy, slug);
    assert_eq!(created["info"]["name"], "Copy");
    let (_, original) = s
        .call("GET", &format!("/api/projects/{slug}/datasets"), None)
        .await;
    let (_, imported) = s
        .call("GET", &format!("/api/projects/{copy}/datasets"), None)
        .await;
    assert_eq!(imported, original);

    for path in [
        format!("/api/projects/{slug}/exports/../project.sqlite"),
        format!("/api/projects/{slug}/exports/nope.am6p"),
    ] {
        let (status, _) = s.call("GET", &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    // Not a project file.
    std::fs::write(s.dir.path().join("bad.am6p"), b"not a zip").expect("write");
    let (status, _) = s
        .command(
            "project.import",
            json!({"package": {"path": s.path("bad.am6p")}}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
