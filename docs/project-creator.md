# Project Creator: source study

Status: **study reviewed, decisions taken** (2026-11-13). No code yet. This document picks the
sources and the access method of the Project Creator, and lists the decisions
left to make. Probes are in `_shared/creator/` (GDAL 3.12 command line, from
macOS, over the public internet).

## Goal

Select a region on a map (rectangle or administrative unit), then get a
working project without leaving AccessMod: DEM, land cover with its table and
a default scenario, population, roads, rivers, zones, and facilities as a
starting point. The work runs as one queued task (progress, pause, cancel),
is tested without the network, and never reads a whole country into memory.

Today users work by hand, per country, or with
[inAccessMod](https://github.com/unige-geohealth/inAccessMod) (R,
[JOSS 2023](https://joss.theoj.org/papers/10.21105/joss.05879), no longer
maintained). Its sources: geoBoundaries, SRTM (`geodata`), WorldPop,
Copernicus Global Land Cover 100 m (CGLS-LC100, no update after 2019),
OpenStreetMap through Geofabrik (`osmextract`), and WHO HeRAMS facilities.

## The constraint that decides most choices

The archives we ship carry their own GDAL (`gdal-src` 0.3, GDAL 3.12, see
`docs/distribution.md`), built with the drivers the code uses only. That
build:

- has **no network access**: curl is off by default. `gdal-src` can link a
  static libcurl (`curl-sys`), but that brings a TLS stack per platform to
  build and maintain;
- **cannot offer** Parquet/Arrow, Zarr or OSM (PBF): `gdal-src` has no
  feature for them. The Homebrew GDAL used in development has all three;
  users would not.

So the sources must be **GeoTIFF/COG, GeoPackage, GeoJSON or FlatGeobuf**,
possibly zipped (`/vsizip/` is built in). Overture Maps (GeoParquet), Zarr
stores and Geofabrik PBF would each need a separate reader in Rust, and are
left for later.

Network access is a **new dependency** either way. Nothing in the workspace
does HTTP today (no `ureq`, `reqwest` or `rustls` in `Cargo.lock`).

## Layers

Measured on South Sudan (SSD) and the window 33.0–33.5° E, 9.0–9.5° N
(550 × 550 cells at 100 m, UTM 36N).

### DEM: Copernicus DEM GLO-90, GLO-30 optional

- **Access:** COG on AWS open data, one tile per 1° × 1°: `copernicus-dem-90m` (4.3 MB a
  tile), `copernicus-dem-30m` (38 MB). Range requests and overviews work.
- **Measured:** the window warped to 100 m in 0.75 s (GLO-90) and 1.0 s (GLO-30).
- **Version:** tiles dated 2022, from the stable 2021 release. Pin the bucket and the
  release in the catalogue.
- **Licence:** free use with attribution (Copernicus DEM licence; GLO-90 fully open).
  GLO-30 coverage of a few countries was restricted in early releases: to verify before
  offering GLO-30 everywhere.
- **Caveat:** it is a surface model (DSM), with tree canopy and buildings included, unlike
  SRTM's terrain. Slopes in forests and towns are affected, so anisotropic walking times
  differ. Say so in the dataset note.

### Land cover: ESA WorldCover 10 m (v200, 2021)

- **Access:** COG on AWS (`esa-worldcover`), one tile per 3° × 3° (134 MB), Byte, NoData 0,
  overviews down to 562².
- **Measured:** the window classified by mode at 100 m in 0.5 s through the overviews, and in
  0.9 s at full resolution (`-ovr NONE`). Class counts differ by under 0.3 %. Use full
  resolution: overviews are made by nearest neighbour, not by mode.
- **Licence:** CC BY 4.0. 11 classes, a fixed legend: the preset land cover table and a
  default scenario (km/h per class, water and wetland as barriers) are written once,
  documented and reviewed.
- **Comparison:** CGLS-LC100, which inAccessMod uses, is no longer produced.

### Population: WorldPop R2025A (constrained, 100 m), or GHS-POP R2023A

- **WorldPop:**
  - one GeoTIFF per country and year (SSD 2025: 20 MB, tiled 512², LZW, Float32, NoData
    −99999);
  - **the server ignores Range requests** (200 with the whole file; GDAL: "Range
    downloading not supported"). So: download the country file, cache it, then warp
    locally;
  - its URL changes between releases (the 2020 path is gone), so pin the release;
  - CC BY 4.0.
- **GHS-POP (JRC):**
  - Mollweide 100 m, 10° tiles zipped (7–40 MB), or a global zip (5.3 GB);
  - range requests work, also inside the zip (`/vsizip//vsicurl/`): the window warped with
    `sum` in 3.5 s;
  - an epoch every 5 years (2025 modelled);
  - CC BY 4.0.
- **Measured:** totals on the window, GHS-POP ≈ 40 500 and WorldPop ≈ 36 700 (WorldPop is
  constrained to settled cells).
- **Recommendation:** WorldPop by default, the reference in the public health field and in
  inAccessMod; GHS-POP as a choice.
- Resample with `sum` and check the total against the source (the 0.5 % criterion of the
  plan).

### Zones and region choice: geoBoundaries (gbOpen)

- **Access:** a JSON API per country and level. Each answer gives download URLs pinned to a
  commit (`…/raw/9469f09/…`), simplified geometries for the map picker, the unit count, the
  source and **the licence, which varies per boundary** (SSD ADM2: CC BY 3.0 IGO). Record it
  per dataset, as the attribution.
- GeoJSON, read as is.
- Same source as inAccessMod.

### Roads, rivers, facilities: OpenStreetMap through the HOT exports

- **Access:** the Humanitarian OpenStreetMap Team publishes per-country GeoPackages, zipped,
  listed on HDX: `production-raw-data-api/ISO3/<ISO3>/<theme>/…_gpkg.zip`. For SSD: roads 49
  MB, waterways 24 MB, health facilities 17 kB. Readable as they are (`/vsizip/` + GPKG).
- **Freshness:** HDX says monthly, but the SSD files on S3 are dated May 2026 while the HDX
  record says November. Read the URL and date from the HDX API (`package_show`), not from a
  fixed path, and show the date.
- **Licence:** ODbL. Attribution, and share-alike for derived databases: say it in the
  dataset notes and the export.
- **Mapping:**
  - roads: OSM `highway` values to AccessMod road classes (a preset table, as for land
    cover);
  - rivers: `waterway=river|canal` to barriers, with the width question open;
  - facilities: `amenity|healthcare`, offered as a starting point and flagged; users import
    their own (HeRAMS, ministry lists).
- **Rejected for now:**
  - **Overpass:** the public server answered "at capacity, request queue full" during the
    probe, and its policy reserves it for human use. Not for country-scale jobs.
  - **Geofabrik PBF:** 139 MB for SSD; needs an OSM reader we do not ship.
  - **Overture Maps:** GeoParquet, monthly releases (2026-10-21.0 latest), ready-made
    geometries and good long-term stability. It needs Parquet in Rust (a large dependency)
    and HTTP range reads. The best long-term candidate for roads: revisit when the
    Creator exists.

## Architecture this suggests

1. **Catalogue**, versioned in the repository (`sources.json`): per layer, URL pattern,
   release, licence, attribution, tiling, NoData, resampling. A source change is a reviewed
   commit, never a silent change.
2. **Fetch:** download what the region needs into a cache shared by the server's projects
   (`<data>/cache/sources/<source>/<release>/…`), keyed by release and tile:
   - whole files where ranges fail (WorldPop, HOT, geoBoundaries);
   - COG tiles where they work.

   The cache makes a second project in the same country, or a retry, free. A cancelled
   fetch leaves only complete files, written to a temporary name and then renamed, with
   size and checksum when the source gives them.
3. **Build:** local files only, with the existing toolkit:
   - `GridSpec` from the region (UTM zone of the centroid, chosen resolution);
   - warp to the grid (a new toolkit block, `raster::warp_to_grid`, with
     mode/bilinear/sum);
   - then `Project::create_with` + `Project::import`, as the demo and the AM5 import do.
   The project appears whole or not at all.
4. **Check:** a travel-time job on the result validates; the population total is compared
   with the source.
5. **Tests without network:** the catalogue points to fixtures, served by a local HTTP server
   in the tests to exercise ranges, 404, timeouts and truncated bodies. One opt-in live test
   per source (`--ignored`) checks that the URLs still answer.

Fetching options:

- **A. Rust HTTP client** (`ureq` + `rustls`, small, no system TLS): downloads whole files
  and byte ranges into the cache; GDAL only ever reads local files. One TLS stack for every
  platform, testable, proxy settings ours. COG range reads would be done by us: per tile,
  the header plus the blocks covering the region.
- **B. GDAL `/vsicurl/`** (static libcurl in the bundle): reads COG windows directly, with
  less code. But a C TLS stack has to be built on each platform. It is also harder to test
  (no fault injection), and caching is GDAL's (in memory, not across runs).

**Recommendation: A**, downloading whole tiles at first. A COG tile is at most 134 MB
(WorldCover); a country needs few of them, and the cache keeps them. Range reads can come
later, without changing the build.

## Scale (order of magnitude, to confirm on a large country)

| Layer | South Sudan (≈ 620 000 km²) | Note |
| --- | --- | --- |
| DEM GLO-90 | ≈ 70 tiles × 4 MB ≈ 300 MB | GLO-30: ≈ 2.7 GB |
| WorldCover | ≈ 10 tiles × 130 MB ≈ 1.3 GB | dominates; ranges would cut it |
| WorldPop | 20 MB | one file |
| HOT roads + waterways + facilities | ≈ 73 MB | zipped GeoPackages |
| geoBoundaries ADM0–2 | < 5 MB | |

At 100 m, the project grid is about 10 000 × 7 000 cells: well within the tiled storage.

## Decisions (2026-11-13)

1. **Network:** option A, a Rust HTTP client (`ureq` + `rustls`) filling the cache;
   GDAL reads local files only.
2. **Population:** WorldPop by default, GHS-POP as a choice.
3. **DEM:** GLO-90 by default, GLO-30 as a choice.
4. **OSM:** HOT exports for v1; Overture later.
5. **Facilities from OSM:** imported as a starting point and flagged in the dataset note
   (incomplete; to be replaced by an official list).
6. **Merge:** the Creator runs Merge land cover with the preset tables, so a travel time
   computes at once.
7. **Offline servers:** the Creator says the sources cannot be reached; a cache filled by
   hand (`cache/sources/`) is read as well.

## Next step

A first slice, without the network: catalogue, cache layout, `raster::warp_to_grid` and the
build from local fixture files, giving a project whose travel time validates. Then the
fetcher (option chosen), then the UI (map, region, layers with licence and size).
