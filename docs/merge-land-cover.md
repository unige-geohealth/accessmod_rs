# Merge land cover

A stack of land-cover rasters, road layers and barrier layers merged into one
land-cover raster on the DEM lattice, ready for travel-time analyses. This is
the AccessMod 5 toolbox "Merge land cover". Status (2026-10-16): implemented
(`accessmod merge-land-cover`) and checked against the AccessMod 5 demo
project. Polygon skeletons for barriers are not implemented.

## AccessMod 5 today

`modules/amAnalysisMergeLandCover/amServer.R`, `tools/R/amStackBarrier.R`,
`tools/R/amBridgeTools.R`:

1. **Stack items** are GRASS rasters made one by one in the UI:
   - *stack land cover*: a copy of the land-cover raster with its class
     labels;
   - *stack road*: one raster per road class, rasterised from the road
     lines with `v.to.rast -d` (densified lines). Each class becomes
     class + 1000 when it is below 1000, unless the user opts out;
   - *stack barrier*: lines (`v.to.rast -d`) or areas (`v.to.rast
     type=area`), value 1. Areas can also be reduced to a skeleton line
     (`r.grow.distance`, `r.buffer`, `r.thin`).
2. **Conflicts**: the classes of all non-barrier items are listed, and a
   class that appears in more than one item blocks the merge. An advanced
   "quick correction" reclassifies an item.
3. **Merge** (`processStack`): the first barrier of the stack becomes a mask
   for every item (`amGrassMaskNS`); each later barrier is added to the mask
   of the items processed after it, that is below it in the stack. `r.patch`
   then keeps, for each cell, the first non-null item in stack order. The
   usual stack: barriers that close everything (conflict areas, destroyed
   bridges) at the top, then roads, then rivers and lakes, then the land
   cover: roads cross rivers, a river without a road stays closed.
4. **Bridges** (option on by default): a road cell is set to null when one
   of its diagonal neighbours is non-null in the merged raster while the two
   cells sharing that corner are null. Such a cell would otherwise let
   travel cross a barrier diagonally. The bridge cells form a separate
   raster.

## accessmod-rs approach

- **One job, one pass.** The stack is a list in the job, so there are no
  intermediate rasters. A road item covers all classes of a layer; the
  order of `classes` sets their priority (one AccessMod 5 stack item per
  class).
- **Rank painting.** Every class has a rank from its position in the stack.
  A cell keeps the class with the lowest rank and records whether a road or
  a barrier touches it (`accessmod_core::merge::MergeCell`). The result does
  not depend on the order the items are painted in.
- **Rasterisation in pixel coordinates of the whole grid**, in
  `accessmod-core` (no GDAL):
  - lines burn every cell holding a point of the line, cells being
    half-open (`[col, col + 1) × [row, row + 1)`): an end point on a cell
    edge burns the cell it belongs to. Each row's span is computed from the
    segment's own end points, in a fixed orientation, so a line burns the
    same cells whatever its direction and the tile or strip boundaries;
  - polygons burn the cells whose centre is inside (even-odd rule, holes
    excluded, half-open edges).
- **Bounded memory.** The merge state lives in the tiled store
  (16 bytes/cell resident, 13 in the scratch files, disk-backed under
  `--memory`). Rasters are read in
  strips, vectors one feature and one part at a time (a part's vertices
  are the only memory not counted in the budget), and outputs are written
  in 256 × 256 blocks with a one-cell halo for the bridge test. Outputs are
  byte-identical at any budget (`tests/merge.rs`).
- **Barriers by position.** A barrier empties the cells it covers in the
  items below it, never in those above: at the top it closes everything,
  between the roads and the land cover it lets roads cross it. This is
  AccessMod 5's rule for every barrier but the first, which AccessMod 5
  applies to the whole stack wherever it is; here the first barrier follows
  the same rule as the others (deviation `barrier_position`). The two agree
  whenever the first barrier is at the top.
  (Until 2026-11-18 every barrier emptied every item. In an older project,
  rivers placed above the roads still cut them: redo the merge with the
  rivers moved below the roads, barriers that must close everything kept at
  the top, and a new output tag.)
- **Explicit reprojection.** Vector layers in another CRS are reprojected
  to the DEM CRS, with a warning in `run.json`. Rasters must already lie on
  the DEM lattice, in its CRS.

## Contract

Job (`accessmod merge-land-cover schema` prints it):

```json
{
  "schema_version": 1,
  "dem": "dem.tif",
  "stack": [
    {"type": "barrier", "path": "conflict_areas.gpkg"},
    {"type": "road", "path": "roads.gpkg", "class_field": "class",
     "label_field": "label", "classes": [1, 2, 3]},
    {"type": "barrier", "path": "rivers.gpkg"},
    {"type": "barrier", "path": "lakes.gpkg", "where": "area_km2 > 1"},
    {"type": "land_cover", "path": "land_cover.tif",
     "labels": [{"class": 1, "label": "forest"}],
     "reclass": [{"from": 7, "to": 70}]}
  ],
  "clean_bridges": true,
  "output_dir": "merged"
}
```

- `road`: `class_field` holds integer classes (integer fields, whole reals
  or integer text). `classes` selects classes and orders them, highest
  priority first; when it is omitted, all classes are used in ascending
  order. `add_1000` (default true) adds 1000 to classes below 1000.
  Features without a class are left out, with a warning.
- `land_cover`: any raster of integer classes on the DEM lattice; there may
  be several. `labels` refer to the raster's classes; `reclass` changes
  classes while merging.
- `barrier`: lines and polygons; multi-geometries and collections are
  split. Points and curves are left out, with a warning. It empties the
  items below it in the stack (above: conflict areas closing roads too;
  below the roads: rivers the roads cross).
- Validation fails when a class comes from more than one item. The message
  names the items, as AccessMod 5 does.
- A feature that cannot be reprojected, or has non-finite coordinates,
  fails the run (exit code 2) with its FID: leaving it out silently could
  open a barrier. Features that burn no cell (outside the grid) are counted
  in `run.json` with a warning.

Outputs:

| File | Content |
| --- | --- |
| `land_cover_merged.tif` | Int32 classes; NoData −2147483648 (barriers, bridges, cells no item covers) |
| `bridges.tif` | Byte, 1 on bridge cells, NoData 0 (only with `clean_bridges`) |
| `classes.csv` | `class,label,stack_item,cells` of the merged raster, for building scenarios |
| `job.resolved.json`, `run.json` | Job with defaults, inputs with SHA-256, per-item classes and feature counts, cell counts, warnings, deviations |

## Comparison with AccessMod 5

`scripts/oracle/merge-land-cover-demo.sh` runs the GRASS commands of the
toolbox on the demo project, in the module's sequence (`processStack`:
the first barrier masks every item, each later one the items below it).
Every case has its first barrier at the top, where AccessMod 5 and this
rule agree (`tests/oracle_merge.rs`):

- both barriers, the river (lines) and the wetland (areas), at the top,
  then the roads (classes 1, 2, 3, both ways) and the land cover: roads are
  cut by the river;
- the wetland at the top, the roads, the river, then the land cover: roads
  cross the river.

| Case | Cells differing (of 44 492) | Bridge map |
| --- | --- | --- |
| roads 1, 2, 3, no cleaning | 2 | — |
| roads 1, 2, 3, cleaned | 2 | identical |
| roads 3, 2, 1, cleaned | 2 | identical |
| river below the roads | 2 | — |
| river below the roads, cleaned | 2 | identical |

The two cells come from line rasterisation. Against the GRASS stack items,
our lines differ on 1 of about 1 320 road cells and 1 of about 1 680 river
cells, where a line passes close to a cell corner. Areas are identical. For
reference, `gdal_rasterize -at` differs on 6 road cells and 1 river cell.

## Deviations

Listed in `run.json` (`deviations`):

- `line_rasterisation`: "all touched" lines computed on the DEM lattice;
  `v.to.rast -d` differs on a few cells near cell corners.
- `stack_rules`: any number of `land_cover` items, no output tag.
- `labels_verbatim`: labels are kept as given; AccessMod 5 lowercases them
  and replaces punctuation with underscores.
- `barrier_position`: every barrier empties only the items below it; AccessMod
  5 applies the first barrier of the stack to every item, wherever it is.

Not implemented: the skeleton option for polygon barriers
(`polyAsSkeleton` in `amAddStackBarrier`).
