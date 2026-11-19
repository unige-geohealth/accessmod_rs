//! Where the sources of the Project Creator live (`docs/project-creator.md`,
//! "Layers"): releases pinned in the URLs, licences and attributions with
//! each asset. A change here is a reviewed commit, never a silent change;
//! [`VERSION`] goes into every project's manifest.

use serde::{Deserialize, Serialize};

use crate::Format;

/// Version of this catalogue.
pub const VERSION: u32 = 2;

/// A region in WGS 84 degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// West longitude.
    pub west: f64,
    /// South latitude.
    pub south: f64,
    /// East longitude.
    pub east: f64,
    /// North latitude.
    pub north: f64,
}

/// A source product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    /// Copernicus DEM GLO-90 (2021 release), 1° tiles.
    CopernicusGlo90,
    /// ESA WorldCover 10 m v200 (2021), 3° tiles.
    WorldCover2021,
    /// WorldPop R2025A, 100 m, constrained, 2025, per country.
    WorldPop2025,
    /// OpenStreetMap through the HOT exports, per country and theme.
    HotOsm,
    /// geoBoundaries gbOpen, per country and level.
    GeoBoundaries,
}

impl Product {
    /// Licence and attribution to keep with the data.
    pub fn attribution(self) -> &'static str {
        match self {
            Product::CopernicusGlo90 => {
                "Copernicus DEM GLO-90 © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH \
                 2014-2018, provided under COPERNICUS by the European Union and ESA; all rights reserved"
            }
            Product::WorldCover2021 => {
                "© ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) \
                 processed by ESA WorldCover consortium (CC BY 4.0)"
            }
            Product::WorldPop2025 => {
                "WorldPop (www.worldpop.org), University of Southampton, R2025A (CC BY 4.0)"
            }
            Product::HotOsm => {
                "© OpenStreetMap contributors (ODbL), exported by the Humanitarian OpenStreetMap Team"
            }
            Product::GeoBoundaries => {
                "geoBoundaries (www.geoboundaries.org), licence per boundary (see the dataset)"
            }
        }
    }
}

/// A file to fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Product.
    pub product: Product,
    /// Key in the cache (relative path, release included).
    pub key: String,
    /// URL.
    pub url: String,
    /// Expected format.
    pub format: Format,
    /// Whether the source may not have it (a tile at sea).
    pub optional: bool,
}

fn lat_name(lat: i32, width: usize) -> String {
    let (h, v) = if lat < 0 { ('S', -lat) } else { ('N', lat) };
    format!("{h}{v:0width$}")
}

fn lon_name(lon: i32, width: usize) -> String {
    let (h, v) = if lon < 0 { ('W', -lon) } else { ('E', lon) };
    format!("{h}{v:0width$}")
}

/// South-west corners of the `step`° tiles covering `b`.
fn tiles(b: &Bounds, step: i32) -> Vec<(i32, i32)> {
    let corner = |v: f64| ((v / step as f64).floor() as i32) * step;
    let (lat0, lat1) = (corner(b.south), corner(b.north - 1e-9));
    let (lon0, lon1) = (corner(b.west), corner(b.east - 1e-9));
    let mut out = Vec::new();
    let mut lat = lat0;
    while lat <= lat1 {
        let mut lon = lon0;
        while lon <= lon1 {
            out.push((lat, lon));
            lon += step;
        }
        lat += step;
    }
    out
}

/// Copernicus DEM GLO-90 tiles over `b` (tiles at sea do not exist).
pub fn dem_tiles(b: &Bounds) -> Vec<Asset> {
    tiles(b, 1)
        .into_iter()
        .map(|(lat, lon)| {
            let name = format!(
                "Copernicus_DSM_COG_30_{}_00_{}_00_DEM",
                lat_name(lat, 2),
                lon_name(lon, 3)
            );
            Asset {
                product: Product::CopernicusGlo90,
                key: format!("copernicus-dem-90m/2021/{name}.tif"),
                url: format!("https://copernicus-dem-90m.s3.amazonaws.com/{name}/{name}.tif"),
                format: Format::Tiff,
                optional: true,
            }
        })
        .collect()
}

/// ESA WorldCover 2021 tiles over `b`.
pub fn land_cover_tiles(b: &Bounds) -> Vec<Asset> {
    tiles(b, 3)
        .into_iter()
        .map(|(lat, lon)| {
            let name = format!(
                "ESA_WorldCover_10m_2021_v200_{}{}_Map",
                lat_name(lat, 2),
                lon_name(lon, 3)
            );
            Asset {
                product: Product::WorldCover2021,
                key: format!("esa-worldcover/v200-2021/{name}.tif"),
                url: format!(
                    "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/{name}.tif"
                ),
                format: Format::Tiff,
                optional: true,
            }
        })
        .collect()
}

/// Whether `iso3` looks like an ISO 3166-1 alpha-3 code (it goes into URLs).
pub fn valid_iso3(iso3: &str) -> bool {
    iso3.len() == 3 && iso3.bytes().all(|b| b.is_ascii_uppercase())
}

/// WorldPop R2025A population of country `iso3` (2025, 100 m, constrained).
pub fn population(iso3: &str) -> Option<Asset> {
    valid_iso3(iso3).then(|| {
        let lower = iso3.to_ascii_lowercase();
        Asset {
            product: Product::WorldPop2025,
            key: format!("worldpop/R2025A/{iso3}.tif"),
            url: format!(
                "https://data.worldpop.org/GIS/Population/Global_2015_2030/R2025A/2025/{iso3}/v1/100m/constrained/{lower}_pop_2025_CN_100m_R2025A_v1.tif"
            ),
            format: Format::Tiff,
            optional: false,
        }
    })
}

/// A theme of the HOT exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Roads (lines).
    Roads,
    /// Waterways (lines).
    Waterways,
    /// Health facilities (points).
    HealthFacilities,
}

/// The HOT export of `theme` for country `iso3` (zipped GeoPackage).
pub fn osm(iso3: &str, theme: Theme) -> Option<Asset> {
    valid_iso3(iso3).then(|| {
        let lower = iso3.to_ascii_lowercase();
        let (name, kind) = match theme {
            Theme::Roads => ("roads", "lines"),
            Theme::Waterways => ("waterways", "lines"),
            Theme::HealthFacilities => ("health_facilities", "points"),
        };
        Asset {
            product: Product::HotOsm,
            key: format!("hotosm/{iso3}/{name}_{kind}.gpkg.zip"),
            url: format!(
                "https://s3.dualstack.us-east-1.amazonaws.com/production-raw-data-api/ISO3/{iso3}/{name}/{kind}/hotosm_{lower}_{name}_{kind}_gpkg.zip"
            ),
            format: Format::Zip,
            optional: true,
        }
    })
}

/// The HOT raw-data API: on-demand OpenStreetMap extracts.
pub const RAW_DATA_API: &str = "https://api-prod.raw-data.hotosm.org/v1";

/// Largest area of one raw-data API request, km²: the API refuses more than
/// 80 000 km² without a login; the margin covers its own area measure.
pub const SNAPSHOT_MAX_KM2: f64 = 70_000.0;

/// Mean Earth radius, km.
const EARTH_RADIUS_KM: f64 = 6_371.008_8;

impl Bounds {
    /// Area on the sphere, km².
    pub fn area_km2(&self) -> f64 {
        let (s, n) = (self.south.to_radians(), self.north.to_radians());
        EARTH_RADIUS_KM.powi(2)
            * (self.east - self.west).to_radians().abs()
            * (n.sin() - s.sin()).abs()
    }

    /// Whether `self` and `other` share some area or an edge.
    pub fn intersects(&self, other: &Bounds) -> bool {
        self.west <= other.east
            && other.west <= self.east
            && self.south <= other.north
            && other.south <= self.north
    }
}

/// `b` cut into the fewest equal n × n tiles of at most
/// [`SNAPSHOT_MAX_KM2`] each, row by row from the south-west.
pub fn snapshot_tiles(b: &Bounds) -> Vec<Bounds> {
    let tiles = |n: usize| -> Vec<Bounds> {
        let dx = (b.east - b.west) / n as f64;
        let dy = (b.north - b.south) / n as f64;
        let mut out = Vec::with_capacity(n * n);
        for row in 0..n {
            for col in 0..n {
                // Edges from the bounds, not by accumulation: tiles abut.
                let edge = |i: usize, lo: f64, hi: f64, d: f64| {
                    if i == n { hi } else { lo + d * i as f64 }
                };
                out.push(Bounds {
                    west: edge(col, b.west, b.east, dx),
                    east: edge(col + 1, b.west, b.east, dx),
                    south: edge(row, b.south, b.north, dy),
                    north: edge(row + 1, b.south, b.north, dy),
                });
            }
        }
        out
    };
    // The whole Earth needs about 85 × 85 tiles; beyond, bounds are not a region.
    for n in 1..=128 {
        let cut = tiles(n);
        if cut.iter().all(|t| t.area_km2() <= SNAPSHOT_MAX_KM2) {
            return cut;
        }
    }
    tiles(128)
}

/// An on-demand extract of the raw-data API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Product (attribution).
    pub product: Product,
    /// Key in the cache.
    pub key: String,
    /// Request body, posted to `{RAW_DATA_API}/snapshot/`.
    pub body: serde_json::Value,
}

/// The OpenStreetMap ferry lines (`route=ferry`) in `tile`, as of
/// `month` (`YYYY-MM`, the cache key: the API serves current data, so a
/// month's extract is reused): a zipped GeoPackage, layer `ferries`, with
/// `osm_id`, `name`, `route` and `highway`; lines are cut at the tile
/// edges.
pub fn ferries(tile: &Bounds, month: &str) -> Snapshot {
    let Bounds {
        west,
        south,
        east,
        north,
    } = *tile;
    Snapshot {
        product: Product::HotOsm,
        key: format!(
            "hotosm/raw-data/ferries/{month}/{west:.5}_{south:.5}_{east:.5}_{north:.5}.gpkg.zip"
        ),
        body: serde_json::json!({
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]],
            },
            "filters": {
                "tags": {"all_geometry": {"join_or": {"route": ["ferry"]}}},
                "attributes": {"all_geometry": ["name", "route", "highway"]},
            },
            "geometryType": ["line"],
            "outputType": "gpkg",
            "fileName": "ferries",
        }),
    }
}

/// The geoBoundaries list of every country (ADM0 metadata).
pub fn countries() -> Asset {
    Asset {
        product: Product::GeoBoundaries,
        key: "geoboundaries/gbOpen/ALL-ADM0.json".to_owned(),
        url: "https://www.geoboundaries.org/api/current/gbOpen/ALL/ADM0/".to_owned(),
        format: Format::Json,
        optional: false,
    }
}

/// The geoBoundaries metadata of country `iso3` at level `level` (0–5).
pub fn boundary(iso3: &str, level: u8) -> Option<Asset> {
    (valid_iso3(iso3) && level <= 5).then(|| Asset {
        product: Product::GeoBoundaries,
        key: format!("geoboundaries/gbOpen/{iso3}-ADM{level}.json"),
        url: format!("https://www.geoboundaries.org/api/current/gbOpen/{iso3}/ADM{level}/"),
        format: Format::Json,
        optional: true,
    })
}

/// The geometry of a boundary from its metadata (`gjDownloadURL`, or
/// `simplifiedGeometryGeoJSON` when `simplified`): pinned to a commit of
/// the geoBoundaries repository.
pub fn boundary_geometry(metadata: &serde_json::Value, simplified: bool) -> Option<Asset> {
    let field = if simplified {
        "simplifiedGeometryGeoJSON"
    } else {
        "gjDownloadURL"
    };
    let url = metadata.get(field)?.as_str()?;
    let id = metadata.get("boundaryID")?.as_str()?;
    let safe = |s: &str| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !url.starts_with("https://github.com/wmgeolab/geoBoundaries/") || !safe(id) {
        return None;
    }
    let suffix = if simplified { "-simplified" } else { "" };
    Some(Asset {
        product: Product::GeoBoundaries,
        key: format!("geoboundaries/gbOpen/{id}{suffix}.geojson"),
        url: url.to_owned(),
        format: Format::Json,
        optional: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_the_bounds_and_are_named_by_their_south_west_corner() {
        let juba = Bounds {
            west: 31.58,
            south: 4.83,
            east: 31.635,
            north: 4.885,
        };
        let dem = dem_tiles(&juba);
        assert_eq!(dem.len(), 1);
        assert!(dem[0].url.ends_with(
            "Copernicus_DSM_COG_30_N04_00_E031_00_DEM/Copernicus_DSM_COG_30_N04_00_E031_00_DEM.tif"
        ));
        let lc = land_cover_tiles(&juba);
        assert!(
            lc[0]
                .url
                .ends_with("ESA_WorldCover_10m_2021_v200_N03E030_Map.tif")
        );
        // Rwanda: south of the equator, across two DEM columns.
        let rwanda = Bounds {
            west: 28.86,
            south: -2.84,
            east: 30.9,
            north: -1.05,
        };
        let names: Vec<String> = dem_tiles(&rwanda).iter().map(|a| a.key.clone()).collect();
        assert_eq!(names.len(), 6, "{names:?}");
        assert!(names.iter().any(|n| n.contains("S03_00_E028_00")));
        assert!(names.iter().any(|n| n.contains("S02_00_E030_00")));
        assert_eq!(
            land_cover_tiles(&rwanda)[0].key,
            "esa-worldcover/v200-2021/ESA_WorldCover_10m_2021_v200_S03E027_Map.tif"
        );
        // A tile edge exactly on the north bound adds no tile.
        let edge = Bounds {
            west: 33.0,
            south: 9.0,
            east: 34.0,
            north: 10.0,
        };
        assert_eq!(dem_tiles(&edge).len(), 1);
    }

    #[test]
    fn snapshot_tiles_stay_under_the_limit_and_cover_the_bounds() {
        let small = Bounds {
            west: 6.55,
            south: 46.38,
            east: 6.70,
            north: 46.52,
        };
        assert_eq!(snapshot_tiles(&small), vec![small]);
        // Tanzania's box, about 1.5 million km² (the API's own figure).
        let tanzania = Bounds {
            west: 29.3,
            south: -11.8,
            east: 40.5,
            north: -0.9,
        };
        assert!((tanzania.area_km2() - 1_501_271.0).abs() / 1_501_271.0 < 0.01);
        let tiles = snapshot_tiles(&tanzania);
        assert_eq!(tiles.len(), 25);
        assert!(tiles.iter().all(|t| t.area_km2() <= SNAPSHOT_MAX_KM2));
        let total: f64 = tiles.iter().map(Bounds::area_km2).sum();
        assert!((total - tanzania.area_km2()).abs() < 1e-6 * total);
        assert_eq!(tiles[0].west, tanzania.west);
        assert_eq!(tiles[24].north, tanzania.north);
        assert_eq!(tiles[0].east, tiles[1].west, "tiles abut");
        let request = ferries(&small, "2026-11");
        assert_eq!(
            request.key,
            "hotosm/raw-data/ferries/2026-11/6.55000_46.38000_6.70000_46.52000.gpkg.zip"
        );
        assert_eq!(request.body["outputType"], "gpkg");
        assert!(!small.intersects(&tiles[0]));
        assert!(tanzania.intersects(&tiles[12]));
    }

    #[test]
    fn countries_go_into_urls_only_when_well_formed() {
        assert!(
            population("SSD")
                .unwrap()
                .url
                .ends_with("/SSD/v1/100m/constrained/ssd_pop_2025_CN_100m_R2025A_v1.tif")
        );
        assert!(
            osm("RWA", Theme::Roads)
                .unwrap()
                .url
                .ends_with("ISO3/RWA/roads/lines/hotosm_rwa_roads_lines_gpkg.zip")
        );
        for bad in ["ss", "SSDX", "../", "ss d", "s/d"] {
            assert!(population(bad).is_none(), "{bad}");
            assert!(osm(bad, Theme::Roads).is_none());
            assert!(boundary(bad, 1).is_none());
        }
        assert!(boundary("RWA", 6).is_none());
        let metadata = serde_json::json!({"boundaryID": "RWA-ADM1-1", "gjDownloadURL": "https://github.com/wmgeolab/geoBoundaries/raw/9469f09/x.geojson"});
        assert!(boundary_geometry(&metadata, false).is_some());
        let hostile =
            serde_json::json!({"boundaryID": "../x", "gjDownloadURL": "https://evil.example/x"});
        assert!(boundary_geometry(&hostile, false).is_none());
    }
}
