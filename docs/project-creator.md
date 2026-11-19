# Project Creator: source study

Status: **experimental in 6.0.0-alpha.1** (2026-11-18): Project › New from a
region and `accessmod project create --recipe`. Presets
(WorldCover classes, OSM road classes and speeds) are provisional; OSM
facilities are a flagged starting point; what is left is in "State on
2026-11-13" below. Study reviewed, decisions taken, contracts written
(2026-11-13). This document picks the
sources and the access method of the Project Creator, and lists the decisions
left to make. Probes are in `_shared/creator/` (GDAL 3.12 command line, from
macOS, over the public internet).

## Goal

Select a region on a map (rectangle or administrative unit), then get a
working project without leaving AccessMod: DEM, land cover with its table and
default scenarios (walking, bicycle, motorized), population, roads, rivers,
zones, and facilities as a starting point. The work runs as one queued task (progress, pause, cancel),
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
- **Caveat:** it is a surface model (DSM): tree canopy and buildings raise it, as they
  partly raise SRTM. Slopes in forests and towns are affected, so anisotropic walking times
  are too. The dataset note says so, with the vertical reference (EGM2008).

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
7. **Offline servers:** the Creator says the sources cannot be reached. Offline packs
   (contract 9) carry verified sources to them.

## Contracts

Written before the code, reviewed by Codex (GPT-6.1-Sol, 2026-11-13). The v1 scope is
narrower than the study:
- **Sources:** GLO-90, WorldCover, WorldPop, geoBoundaries and the HOT exports, downloaded as
  whole files.
- **Deferred:** GLO-30, GHS-POP, COG range reads and Overture.

### 1. Region

Unlike inAccessMod, which masks every raster at the political border, three things are kept
apart.

- **Area of interest (AOI):** geoBoundaries units picked on the map (country, ADM1, ADM2), a
  rectangle or polygon drawn on the map, or an uploaded Polygon/MultiPolygon (GeoJSON,
  GeoPackage, zipped shapefile). An uploaded border, e.g. a ministry's, replaces
  geoBoundaries and keeps its own provenance.
- **Traversable area:** the AOI buffered by `buffer_m` (metres, in the project CRS). A cell is
  kept when its centre lies inside, so holes and islands follow the polygon. The grid is
  this area's rectangular envelope. The land cover is NoData outside it, which is impassable
  in AccessMod, so every analysis stays inside. The mask is applied **after** Merge land
  cover, so roads cannot reopen masked cells.
- **Reporting zones:** the AOI's units, imported as a zone dataset for statistics.

A buffer reduces border effects. It does not guarantee that the true fastest route stays
inside: the documentation says so, and a buffer of 0 reproduces inAccessMod. A border only
moves a buffer: a disputed line changes what is reported, not what is computable.

### 2. Across borders

- DEM and land cover tiles are selected geometrically, over the traversable area plus the
  resampling kernel's margin.
- WorldPop and HOT come per country: every country the traversable area touches is fetched.
  Countries are found from geoBoundaries ADM0, simplified.
- Coverage that is missing or not fetched shows in the plan and blocks creation, unless the
  user explicitly excludes it.
- OSM features are deduplicated by `osm_id`. WorldPop country rasters must not overlap; this
  is checked, never assumed.

### 3. Projection and lattice

AccessMod 6 is as strict as AccessMod 5 about the grid:
- one projected CRS in metres (`grid::check_projected_metric`), north-up, square cells;
- every imported raster on the same lattice: same resolution, and the same origin modulo the
  cell size.

Any metric projected CRS is accepted. The plan measures over the traversable extent, edges
included:
- **distance scale distortion**, which drives travel times: the solver uses one cell size
  everywhere;
- **area distortion**, which drives areas and densities. People counts are preserved by the
  `sum` warp whatever the CRS (contract 4).

The two families of projection trade differently:
- **Conformal** (UTM, Transverse Mercator, Lambert Conformal Conic, the Swiss oblique Mercator
  LV95 EPSG:2056, most national grids): the scale error is the same in every direction and
  small near the centre. Best for travel time.
- **Equal area** (Lambert Azimuthal Equal Area, Albers Equal Area): areas are exact, but
  distances stretch one way and shrink the other. Fine on moderate extents, visible on wide
  ones.

Choices, in order of precedence:
1. **Match a reference raster** (or another project): its CRS and lattice (origin,
   resolution) are reused, so national rasters later import without resampling.
2. **A national or user CRS**, by EPSG code or WKT.
3. **Automatic:** UTM when the extent fits one zone. Otherwise a Transverse Mercator centred on
   the extent, or Albers/LAEA when the user prefers exact areas.

The plan shows both distortions. Past 0.5 % of distance distortion it warns; past 2 % it
refuses and suggests splitting the region (the limits are to be confirmed by tests).
Without a reference raster, the lattice origin is snapped to a multiple of the resolution.
Antimeridian crossings and |latitude| > 80° are refused in v1.

### 4. Population

- People per cell, accumulated in Float64. NoData is not zero.
- Source and grid totals are compared over the same footprint, before masking. What the
  mask removes is reported separately.
- Past the tolerance (0.5 %, to be confirmed on synthetic distributions in several CRSs),
  the build fails and gives the figures. It never renormalises silently.
- Source overviews are not used.

### 5. Mosaic and warp with the shipped GDAL

The shipped GDAL opens no VRT, so `raster::warp_to_grid` works per output chunk. For each
chunk, it:
1. copies the source window, plus the kernel margin, from every tile that intersects it into
   one in-memory (MEM) mosaic;
2. runs a single `GDALWarp` from that mosaic to the chunk.

Memory stays bounded and tile seams cannot show. Resamplings: nearest, mode (land cover, at
full resolution), bilinear (DEM), sum (population).

### 6. Presets are scientific assumptions

The presets are versioned data files, documented and shown in the plan and in the dataset
notes: `crates/accessmod-io/src/creator/presets/`.

- **WorldCover:**
  - labels and a default walking scenario;
  - permanent water at 0 km/h;
  - wetland and mangroves to be reviewed;
  - ocean and areas without coverage NoData, never inferred from a boundary.

  Mode aggregation can remove narrow water. Rivers therefore come from OSM barriers. The
  Merge stack is: the outside of the region (closes everything), roads, rivers, land cover.
  A barrier empties only the items below it, so roads cross rivers and WorldCover water
  where they pass, while a river without a road stays closed; Merge's bridge cleaning
  (AccessMod 5's rule) removes diagonal leaks.
- **OSM roads:**
  - `highway` values to road classes, labels and a speed per default scenario;
  - explicit rules for unknown values, `track` and `path`, `access=no`, construction and
    tunnels.
- **Default scenarios** (box ticked by default; `default_scenarios` in the recipe): three
  scenario tables (`tScenario`, tags `creator walking|bicycle|motorized`), to load in
  Analysis › Accessibility, and the matching saved travel-time jobs (`project jobs --json`
  or `GET /api/projects/{project}/jobs`, replayed with `project run travel-time --job`). Off the roads people walk
  (WorldCover speeds). Unticked, the jobs are still checked for readiness, nothing is saved.
- **Expected outputs:** a creation that misses an output it was asked for (a layer absent
  upstream, zones, merged land cover and its table, scenario tables) fails, never a silent
  gap. A layer empty in the region (no road, river or facility) is a warning; no facility
  also leaves the project not ready.

  | Road class | Walking | Bicycle | Motorized |
  | --- | --- | --- | --- |
  | 1 Motorway and trunk | walking 0.1 | walking 0.1 | motorized 80 |
  | 2 Primary, 3 Secondary, 4 Tertiary, 5 Local | walking 5 | bicycling 15 | motorized 60, 50, 40, 30 |
  | 6 Track | walking 5 | bicycling 10 | motorized 15 |
  | 7 Path | walking 5 | walking 5 | walking 5 |

  Motorways and trunk roads are crossed on foot or by bicycle, hardly followed (about one
  hour per 100 m cell; still a way over a bridge with no other): 0.1 km/h, not 0, because a road at 0 km/h would be a barrier: a wall in the merged land cover, and
  facilities on it skipped.
- **OSM facilities:**
  - points, and polygon centroids, with `osm_id`;
  - no capacity invented, no snapping;
  - facilities on barriers listed;
  - flagged as a starting point, to be replaced by an official list.
- **DEM:** a surface model, like SRTM. Its vertical reference and its slope limits under
  canopy and buildings are stated.

### 7. Created, then ready

- A project is **created** once its inputs are imported.
- It is **ready** once Merge has run and a travel-time quick check is valid.
- It is published only once ready. Otherwise the task fails with actionable reasons; the
  verified cache objects are kept, and the staging directory is removed.

### 8. Provenance: pinned bytes, not names

An immutable **manifest** records:
- the catalogue version, product and year;
- each asset with its resolved URL (secrets redacted), SHA-256, size and provider date;
- licences and attributions;
- the AOI hash, CRS (WKT2), lattice origin, resolution and size;
- warp settings and preset versions.

It is stored in the project and in the Creator run's `run.json`. A replay from a manifest
refuses missing or different bytes and never takes "latest". Licences and attributions
travel into project files and data archives:
- geoBoundaries licences are per boundary;
- OpenStreetMap is ODbL, and its share-alike note goes with merged derivatives.

Borders are described neutrally.

### 9. Download and cache

- **Downloads:** streamed to a temporary file. Length, digest (when the provider gives one)
  and format (magic bytes, so a 200 HTML page is refused) are checked before an atomic
  rename.
- **Failures handled:** 429 and 503 with backoff, redirects, truncation, disk full,
  cancellation. Resume only with validators and a correct 206; otherwise download again.
- **Cache:** `<data>/cache/sources/`, a store of immutable objects:
  - a lock per object, and leases while a build uses one;
  - a configurable quota with LRU eviction;
  - projects never link into it.
- **HOT zips:** their GeoPackage members are extracted to disk with bounds (expected names,
  sizes).
- **Offline packs:** a verified set of cache objects (manifest and files) can be exported and
  imported. An offline mode lists the missing assets without contacting anyone.

### 10. Networking

- `ureq` with `rustls` and the platform's certificate verifier, so corporate CAs work.
  Verification is never off.
- `HTTPS_PROXY` / `NO_PROXY`, with documented precedence, and Basic proxy authentication.
- Diagnostics redact secrets.
- Tested on the packaged targets, and with the network off.

### 11. Layers

- HTTP lives in a new crate, `accessmod-fetch`. `accessmod-io` stays free of HTTP, and
  `cargo xtask layers` checks it.
- `accessmod_io::creator` builds from local files and a manifest.
- The CLI and the server compose the two through the command registry: UI, CLI and job
  file reach the same commands.

## Slices

Each slice passes every gate and gets a Codex review before the next one.

Done: 1 (contracts, presets); 2 (`accessmod_io::creator`, `accessmod project create --recipe`; tested on real sources cut around Juba: the population over the grid equals the sources over the same footprint to 1e-8, outputs are identical across memory budgets and thread counts). The outside of the buffered region is a barrier at the top of the Merge stack, so the merge job itself keeps everything inside (replayable), rather than a mask applied afterwards. After the Codex review of slice 2: hostile tile origins and coarse resolutions are refused, the mosaic halo follows the kernel scale, overlapping tiles must agree, a missing DEM or land cover tile under the region refuses the build, invalid area polygons are refused, a project without facilities is not created (readiness), facility polygons keep their own centroid. Left to later slices: population coverage per country against geoBoundaries ADM0 and licences/attributions (slice 5, with the catalogue), the Creator run record (`run.json`, slice 6).

State on 2026-11-13: a first usable flow, end to end.

- **Engine:** only the DEM is required; land cover, population, OSM roads,
  rivers and facilities are optional. Facilities come from a list of your
  own (kept whole, with its id field) or from OpenStreetMap (filtered and
  flagged). Readiness is reported (`ready`, `not_ready` in the
  manifest), not required.
- **`accessmod-fetch`:** HTTPS (rustls, platform certificates, proxies,
  retries), a cache of checked files (`<data>/cache/sources/`), the
  catalogue (GLO-90 and WorldCover tiles, WorldPop and HOT per country,
  geoBoundaries).
- **Server:** `creator.countries`, `creator.units`, `creator.create` (a
  queued task: fetch, then build; outcome says ready or what is missing).
- **Client:** Project › New from a region: country, level, units clicked on
  the map, passable distance, cell size, layers.
- **Tested:**
  - live, Juba county: 972 × 885 cells at 200 m, 1 091 948 people equal to
    the source to 1e-9, 8 847 roads, 226 facilities, ready, 70 s with
    downloads;
  - e2e: from a cache filled beforehand (`web/e2e/fixtures/sources`), no
    network.

Still to do:
- slice 3: cache quota and eviction, offline pack export and import;
- slice 5: population coverage per country across borders (ADM0);
- slice 6:
  - a preflight plan (download size, grid size, distortion) before Create;
  - a drawn or uploaded region;
  - a facility list upload in the form;
  - the Creator run record.

1. These contracts and the presets.
2. Build from local files (`accessmod_io::creator`):
   - recipe and catalogue;
   - `GridSpec::from_extent`, the projection check, `warp_to_grid`, the vector clip and
     the mask;
   - Merge, readiness, manifest;
   - CLI `accessmod project create --recipe … --offline`.
3. Cache and offline packs (no HTTP).
4. `accessmod-fetch` (`ureq` + `rustls`), with fault injection from a local server.
5. Provider adapters, resolved into manifests; live probes outside the release gate.
6. Server commands (`creator.plan` preflight, `creator.create` task) and the UI
   (Project › New › From a region: map selection or drawing, buffer, resolution, preflight).
7. Release qualification: five packaged targets, network off, 2 GB machines, live regions
   (South Sudan county, Rwanda, a coastal cross-border area, a refused wide area), recovery
   after the process is killed, binary size.
