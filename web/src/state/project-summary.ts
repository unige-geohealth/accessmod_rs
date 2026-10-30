// Project summary selectors (Project › Project summary): grid parameters and
// extents, from the project grid and the DEM bounds. AM5 shows them with
// GRASS names (`g.region`), which stay untranslated.
import type { Api } from "../generated/types";

export type Entry = [name: string, value: string];

const metres = (v: number) => String(Math.round(v * 1000) / 1000);
const degrees = (v: number) => v.toFixed(6);

export function gridParameters(grid: Api.GridInfo): Entry[] {
  const [, ewres = 0, , , , nsres = 0] = grid.geo_transform;
  return [
    ["nsres", metres(Math.abs(nsres))],
    ["ewres", metres(Math.abs(ewres))],
    ["rows", String(grid.rows)],
    ["cols", String(grid.cols)],
    ["cells", String(grid.rows * grid.cols)],
  ];
}

/** North, south, east and west in the project CRS (north-up grids). */
export function extentMetric(grid: Api.GridInfo): Entry[] {
  const [x0 = 0, dx = 0, , y0 = 0, , dy = 0] = grid.geo_transform;
  const xs = [x0, x0 + grid.cols * dx];
  const ys = [y0, y0 + grid.rows * dy];
  return [
    ["north", metres(Math.max(...ys))],
    ["south", metres(Math.min(...ys))],
    ["east", metres(Math.max(...xs))],
    ["west", metres(Math.min(...xs))],
  ];
}

/** From `GET /bounds`: `[west, south, east, north]` in WGS 84. */
export function extentDegrees([west, south, east, north]: readonly [number, number, number, number]): Entry[] {
  return [
    ["north", degrees(north)],
    ["south", degrees(south)],
    ["east", degrees(east)],
    ["west", degrees(west)],
  ];
}

/** The minimum length of a project name (AM5 `project_new_name_text`). */
export const MIN_NAME = 4;

export const validName = (name: string) => name.trim().length >= MIN_NAME;
