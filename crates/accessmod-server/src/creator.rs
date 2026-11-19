//! Project Creator commands: countries and administrative units to pick a
//! region, then a queued task that fetches the sources into the shared
//! cache (`accessmod_fetch`) and builds the project from local files
//! (`accessmod_io::creator`).

use std::path::{Path, PathBuf};

use accessmod_fetch::catalogue::{self, Asset, Bounds, Theme};
use accessmod_fetch::{Cache, Client, Fetched, Settings};
use accessmod_io::analysis::Monitor;
use accessmod_io::creator::{self, Area, Crs, FacilitySource, Recipe, Sources};
use accessmod_io::error::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::Principal;
use crate::error::{ApiError, ApiResult};
use crate::operations::{self, PendingOperation};
use crate::state::AppState;
use crate::uploads::{self, FileSource};

/// Directory of the shared source cache, under the data directory.
pub const SOURCES_DIR: &str = "cache/sources";

/// Progress stage: downloading the sources.
pub const STAGE_FETCH: &str = "fetch";

fn cache(state: &AppState) -> Cache {
    Cache::new(state.config().data_dir.join(SOURCES_DIR))
}

fn fetch_error(error: accessmod_fetch::Error) -> Error {
    match error {
        accessmod_fetch::Error::Cancelled => Error::Cancelled,
        other => Error::invalid(other.to_string()),
    }
}

/// Fetch `asset` into the cache (or take the cached copy).
fn get(
    state: &AppState,
    asset: &Asset,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Fetched, accessmod_fetch::Error> {
    let client = Client::new(Settings::default());
    cache(state).get(
        &client,
        &asset.key,
        &asset.url,
        asset.format,
        false,
        cancelled,
        progress,
    )
}

fn read_json(path: &Path) -> ApiResult<Value> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ApiError::Internal(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| ApiError::Engine(Error::invalid(format!("{}: {e}", path.display()))))
}

// ---- countries and units ----------------------------------------------------

/// `creator.countries`: no parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Countries {}

/// A country.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Country {
    /// ISO 3166-1 alpha-3 code.
    pub iso3: String,
    /// Name.
    pub name: String,
    /// Continent.
    pub continent: String,
}

/// The countries of geoBoundaries, by name.
pub fn countries(state: &AppState, _: &Principal, _: Countries) -> ApiResult<Vec<Country>> {
    let fetched = get(state, &catalogue::countries(), &|| false, &mut |_, _| {})
        .map_err(|e| ApiError::Engine(fetch_error(e)))?;
    let list = read_json(&fetched.path)?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let mut out: Vec<Country> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| catalogue::valid_iso3(&text(c, "boundaryISO")))
        .map(|c| Country {
            iso3: text(c, "boundaryISO"),
            name: text(c, "boundaryName"),
            continent: text(c, "Continent"),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// `creator.units`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Units {
    /// Country (ISO 3166-1 alpha-3).
    pub iso3: String,
    /// Administrative level, 0 (the country) to 5.
    pub level: u8,
}

fn boundary_metadata(state: &AppState, iso3: &str, level: u8) -> ApiResult<Value> {
    let asset = catalogue::boundary(iso3, level)
        .ok_or_else(|| ApiError::BadRequest(format!("no country {iso3:?} or level {level}")))?;
    let fetched = get(state, &asset, &|| false, &mut |_, _| {}).map_err(|e| match e {
        accessmod_fetch::Error::Absent(_) => {
            ApiError::NotFound(format!("geoBoundaries has no level {level} for {iso3}"))
        }
        other => ApiError::Engine(fetch_error(other)),
    })?;
    read_json(&fetched.path)
}

/// The units of `iso3` at `level`, simplified, as a GeoJSON
/// FeatureCollection with `id` and `name` (and the boundary's licence).
pub fn units(state: &AppState, _: &Principal, params: Units) -> ApiResult<Value> {
    let metadata = boundary_metadata(state, &params.iso3, params.level)?;
    let asset = catalogue::boundary_geometry(&metadata, true)
        .ok_or_else(|| ApiError::Internal("geoBoundaries gave no simplified geometry".into()))?;
    let fetched = get(state, &asset, &|| false, &mut |_, _| {})
        .map_err(|e| ApiError::Engine(fetch_error(e)))?;
    let geojson = read_json(&fetched.path)?;
    Ok(serde_json::json!({
        "type": "FeatureCollection",
        "licence": metadata.get("boundaryLicense").cloned().unwrap_or(Value::Null),
        "source": metadata.get("boundarySource").cloned().unwrap_or(Value::Null),
        "features": features(&geojson).into_iter().map(|f| serde_json::json!({
            "type": "Feature",
            "properties": {"id": unit_id(f), "name": unit_name(f)},
            "geometry": f.get("geometry").cloned().unwrap_or(Value::Null),
        })).collect::<Vec<_>>(),
    }))
}

fn features(geojson: &Value) -> Vec<&Value> {
    geojson
        .get("features")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn unit_id(feature: &Value) -> String {
    feature
        .pointer("/properties/shapeID")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn unit_name(feature: &Value) -> String {
    feature
        .pointer("/properties/shapeName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

// ---- create -----------------------------------------------------------------

/// Layers to fetch.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    /// ESA WorldCover land cover.
    #[serde(default = "yes")]
    pub land_cover: bool,
    /// WorldPop population.
    #[serde(default = "yes")]
    pub population: bool,
    /// OpenStreetMap roads.
    #[serde(default = "yes")]
    pub roads: bool,
    /// OpenStreetMap rivers and canals (barriers).
    #[serde(default = "yes")]
    pub waterways: bool,
    /// OpenStreetMap health facilities (a starting point), when no list
    /// of your own is given.
    #[serde(default = "yes")]
    pub osm_facilities: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            land_cover: true,
            population: true,
            roads: true,
            waterways: true,
            osm_facilities: true,
        }
    }
}

fn yes() -> bool {
    true
}

/// A facility list of your own.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FacilityList {
    /// The layer: an upload, or a file on the server's disk (desktop).
    pub file: FileSource,
    /// Field identifying facilities.
    pub id_field: String,
}

/// `creator.create`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateFromRegion {
    /// Project name.
    pub name: String,
    /// Country (ISO 3166-1 alpha-3): its population and OpenStreetMap layers.
    pub iso3: String,
    /// Administrative level of `units`.
    #[serde(default)]
    pub level: u8,
    /// Units (geoBoundaries `shapeID`) forming the region; all of the level
    /// when empty.
    #[serde(default)]
    pub units: Vec<String>,
    /// A drawn or uploaded region (GeoJSON, WGS 84), instead of units.
    #[serde(default)]
    pub area: Option<Value>,
    /// Passable distance around the region, metres.
    #[serde(default = "ten_km")]
    pub buffer_m: f64,
    /// Cell size, metres.
    #[serde(default = "hundred_m")]
    pub resolution_m: f64,
    /// Layers to fetch.
    #[serde(default)]
    pub layers: Layers,
    /// Facilities of your own (replace OpenStreetMap's).
    #[serde(default)]
    pub facilities: Option<FacilityList>,
    /// Save the travel-time jobs of the preset scenarios (walking, bicycle,
    /// motorized).
    #[serde(default = "yes")]
    pub default_scenarios: bool,
}

fn ten_km() -> f64 {
    10_000.0
}

fn hundred_m() -> f64 {
    100.0
}

impl crate::commands::Scoped for Countries {
    fn project(&self) -> Option<&str> {
        None
    }
}

impl crate::commands::Scoped for Units {
    fn project(&self) -> Option<&str> {
        None
    }
}

impl crate::commands::Scoped for CreateFromRegion {
    fn project(&self) -> Option<&str> {
        None
    }
}

/// Every `[lon, lat]` of a GeoJSON geometry.
fn extend_bounds(value: &Value, b: &mut Bounds) {
    match value {
        Value::Array(items) => {
            if let [Value::Number(x), Value::Number(y), ..] = items.as_slice()
                && let (Some(x), Some(y)) = (x.as_f64(), y.as_f64())
            {
                b.west = b.west.min(x);
                b.east = b.east.max(x);
                b.south = b.south.min(y);
                b.north = b.north.max(y);
            } else {
                items.iter().for_each(|v| extend_bounds(v, b));
            }
        }
        Value::Object(map) => {
            if let Some(c) = map.get("coordinates") {
                extend_bounds(c, b);
            } else if let Some(g) = map.get("geometry") {
                extend_bounds(g, b);
            } else if let Some(f) = map.get("features") {
                extend_bounds(f, b);
            }
        }
        _ => {}
    }
}

/// The region of `params` as GeoJSON features with names.
fn region(
    state: &AppState,
    params: &CreateFromRegion,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Value, String), Error> {
    if let Some(area) = &params.area {
        return Ok((area.clone(), "drawn area".to_owned()));
    }
    let metadata = boundary_metadata(state, &params.iso3, params.level)
        .map_err(|e| Error::invalid(e.to_string()))?;
    let asset = catalogue::boundary_geometry(&metadata, false)
        .ok_or_else(|| Error::invalid("geoBoundaries gave no geometry"))?;
    let fetched = get(state, &asset, cancelled, &mut |_, _| {}).map_err(fetch_error)?;
    let geojson = read_json(&fetched.path).map_err(|e| Error::invalid(e.to_string()))?;
    let selected: Vec<Value> = features(&geojson)
        .into_iter()
        .filter(|f| params.units.is_empty() || params.units.contains(&unit_id(f)))
        .map(|f| {
            serde_json::json!({
                "type": "Feature",
                "properties": {"name": unit_name(f)},
                "geometry": f.get("geometry").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    if selected.is_empty() {
        return Err(Error::invalid(
            "none of the chosen units exists at this level",
        ));
    }
    let licence = metadata
        .get("boundaryLicense")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let label = format!(
        "{} unit(s) of {} ADM{} (geoBoundaries, {licence})",
        selected.len(),
        params.iso3,
        params.level
    );
    Ok((
        serde_json::json!({"type": "FeatureCollection", "features": selected}),
        label,
    ))
}

/// The cached GeoPackage of a HOT export (its zip extracted once, beside it).
fn hot_layer(
    fetched: &Fetched,
    cancelled: accessmod_io::analysis::Cancelled<'_>,
) -> Result<PathBuf, Error> {
    let dir = PathBuf::from(format!("{}.d", fetched.path.display()));
    if !dir.is_dir() {
        let staging = PathBuf::from(format!("{}.tmp-{}", dir.display(), std::process::id()));
        accessmod_io::archive::extract_plain(
            &fetched.path,
            &staging,
            accessmod_io::archive::Limits::default(),
            Monitor::new(cancelled, &|_| {}),
        )?;
        if std::fs::rename(&staging, &dir).is_err() {
            // Another task extracted it meanwhile.
            let _ = std::fs::remove_dir_all(&staging);
        }
    }
    std::fs::read_dir(&dir)
        .map_err(|e| Error::invalid(format!("{}: {e}", dir.display())))?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
        })
        .ok_or_else(|| Error::invalid(format!("{}: no GeoPackage in the export", fetched.url)))
}

/// Fetch the sources of `params` and build the project into `target`.
fn build(
    state: &AppState,
    params: &CreateFromRegion,
    facilities: Option<(PathBuf, String)>,
    target: &Path,
    monitor: Monitor<'_>,
) -> Result<(accessmod_io::project::Project, String), Error> {
    let cancelled = monitor.cancelled();
    monitor.stage(STAGE_FETCH);
    let (area, label) = region(state, params, cancelled)?;
    let mut b = Bounds {
        west: f64::INFINITY,
        south: f64::INFINITY,
        east: f64::NEG_INFINITY,
        north: f64::NEG_INFINITY,
    };
    extend_bounds(&area, &mut b);
    if !(b.west.is_finite() && b.north.is_finite()) {
        return Err(Error::invalid("the region has no coordinates"));
    }
    // The buffer, in degrees, generously (tiles are cheap to skip).
    let lat = b.north.abs().max(b.south.abs()).min(80.0).to_radians();
    let margin = params.buffer_m / 111_000.0 * 1.2 + 0.05;
    let b = Bounds {
        west: b.west - margin / lat.cos(),
        east: b.east + margin / lat.cos(),
        south: b.south - margin,
        north: b.north + margin,
    };
    let mut assets: Vec<(&'static str, Asset)> = catalogue::dem_tiles(&b)
        .into_iter()
        .map(|a| ("dem", a))
        .collect();
    if params.layers.land_cover {
        assets.extend(
            catalogue::land_cover_tiles(&b)
                .into_iter()
                .map(|a| ("land_cover", a)),
        );
    }
    let iso = &params.iso3;
    let wanted = [
        (
            params.layers.population,
            "population",
            catalogue::population(iso),
        ),
        (
            params.layers.roads,
            "roads",
            catalogue::osm(iso, Theme::Roads),
        ),
        (
            params.layers.waterways,
            "waterways",
            catalogue::osm(iso, Theme::Waterways),
        ),
        (
            params.layers.osm_facilities && facilities.is_none(),
            "facilities",
            catalogue::osm(iso, Theme::HealthFacilities),
        ),
    ];
    for (on, layer, asset) in wanted {
        if on {
            assets.push((
                layer,
                asset.ok_or_else(|| Error::invalid(format!("no country {iso:?}")))?,
            ));
        }
    }
    let total = assets.len() as u64;
    let mut got: Vec<(&'static str, Fetched)> = Vec::new();
    let mut absent = Vec::new();
    for (i, (layer, asset)) in assets.iter().enumerate() {
        monitor.advance(STAGE_FETCH, i as u64, Some(total));
        match get(state, asset, cancelled, &mut |_, _| {}) {
            Ok(f) => got.push((layer, f)),
            Err(accessmod_fetch::Error::Absent(_)) if asset.optional => {
                absent.push(asset.key.clone())
            }
            Err(e) => return Err(fetch_error(e)),
        }
    }
    monitor.advance(STAGE_FETCH, total, Some(total));
    // A layer asked for and absent upstream is refused, never a project
    // that silently lacks it.
    let asked: Vec<&str> = assets.iter().map(|(l, _)| *l).collect();
    let fetched: Vec<&str> = got.iter().map(|(l, _)| *l).collect();
    let lacking = unfetched(&asked, &fetched);
    if !lacking.is_empty() {
        return Err(Error::invalid(format!(
            "nothing to download in this region for: {}",
            lacking.join(", ")
        )));
    }
    let files = |layer: &str| {
        got.iter()
            .filter(|(l, _)| *l == layer)
            .map(|(_, f)| f.path.clone())
            .collect::<Vec<_>>()
    };
    let hot = |layer: &str| -> Result<Option<PathBuf>, Error> {
        got.iter()
            .find(|(l, _)| *l == layer)
            .map(|(_, f)| hot_layer(f, cancelled))
            .transpose()
    };
    let mut attributions: Vec<String> = assets
        .iter()
        .filter(|(l, a)| got.iter().any(|(gl, f)| gl == l && f.url == a.url))
        .map(|(_, a)| a.product.attribution().to_owned())
        .collect();
    attributions.dedup();
    attributions.push(label);
    let recipe = Recipe {
        schema_version: 1,
        name: params.name.clone(),
        area: Area::Geojson(area),
        buffer_m: params.buffer_m,
        resolution_m: params.resolution_m,
        crs: Crs::Auto,
        sources: Sources {
            dem: files("dem"),
            land_cover: files("land_cover"),
            land_cover_worldcover: true,
            population: files("population"),
            roads: hot("roads")?,
            waterways: hot("waterways")?,
            facilities: match facilities {
                Some((path, id_field)) => Some(FacilitySource::List { path, id_field }),
                None => hot("facilities")?.map(FacilitySource::Osm),
            },
            urls: got
                .iter()
                .map(|(_, f)| {
                    (
                        f.path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        f.url.clone(),
                    )
                })
                .collect(),
            attributions,
        },
        default_scenarios: params.default_scenarios,
    };
    if recipe.sources.dem.is_empty() {
        return Err(Error::invalid("no DEM tile covers this region"));
    }
    let (project, manifest) = creator::build(&recipe, target, &state.config().settings, monitor)?;
    let outcome = if manifest.ready {
        format!("created, ready to compute ({})", manifest.crs_name)
    } else {
        format!(
            "created ({}); to compute: {}",
            manifest.crs_name,
            manifest.not_ready.join("; ")
        )
    };
    Ok((project, outcome))
}

/// `creator.create`: queue the creation of a project from a region.
pub fn create(
    state: &AppState,
    principal: &Principal,
    params: CreateFromRegion,
) -> ApiResult<PendingOperation> {
    if !catalogue::valid_iso3(&params.iso3) {
        return Err(ApiError::BadRequest(format!(
            "no country {:?}",
            params.iso3
        )));
    }
    if params.name.trim().is_empty() {
        return Err(ApiError::BadRequest("a project needs a name".into()));
    }
    let facilities = match &params.facilities {
        Some(list) => Some((
            uploads::resolve(state, principal, &list.file)?,
            list.id_field.clone(),
        )),
        None => None,
    };
    let slug = crate::state::new_slug_except(&state.projects_dir(), &params.name, &|s| {
        state.pending().reserves(s)
    });
    let job = serde_json::json!({
        "name": params.name, "iso3": params.iso3, "level": params.level, "units": params.units.len(),
        "buffer_m": params.buffer_m, "resolution_m": params.resolution_m, "layers": params.layers,
        "default_scenarios": params.default_scenarios,
    });
    let name = params.name.clone();
    let target = slug.clone();
    operations::queue_pending(
        state,
        principal,
        "project-create",
        &slug,
        &name,
        job,
        move |state, monitor| {
            let list = facilities
                .as_ref()
                .map(|(r, id)| (r.path.clone(), id.clone()));
            let (project, outcome) = build(
                state,
                &params,
                list,
                &state.projects_dir().join(&target),
                monitor,
            )?;
            if let Some((resolved, _)) = facilities {
                resolved.consumed();
            }
            Ok((project, outcome))
        },
    )
}

/// Layers of `asked` with no file in `fetched`, each once, in order.
fn unfetched<'a>(asked: &[&'a str], fetched: &[&str]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for layer in asked {
        if !fetched.contains(layer) && !out.contains(layer) {
            out.push(layer);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::unfetched;

    #[test]
    fn a_layer_asked_and_absent_is_reported() {
        // Two land cover tiles, both absent upstream (ocean): reported once.
        let asked = ["dem", "land_cover", "land_cover", "roads"];
        assert_eq!(unfetched(&asked, &["dem", "roads"]), ["land_cover"]);
        // One tile of two is enough.
        assert!(unfetched(&asked, &["dem", "land_cover", "roads"]).is_empty());
    }
}
