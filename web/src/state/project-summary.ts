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

/**
 * Options of a project select: each project by its name, and by its slug
 * too when another has the same name (made before names were unique, or
 * the demo built again after a project took its name).
 */
export function projectOptions(projects: readonly { slug: string; info: { name: string } }[] | undefined): { value: string; label: string }[] {
  const key = (name: string) => name.trim().toLowerCase();
  const count = new Map<string, number>();
  for (const p of projects ?? []) count.set(key(p.info.name), (count.get(key(p.info.name)) ?? 0) + 1);
  return (projects ?? []).map((p) => ({
    value: p.slug,
    label: (count.get(key(p.info.name)) ?? 0) > 1 ? `${p.info.name.trim()} (${p.slug})` : p.info.name,
  }));
}

/**
 * Whether `name` is taken (AM5 "Project %s is not available."): by a
 * project, or by an unfinished operation making one. Trimmed and
 * case-folded, as the server compares them.
 */
export function nameTaken(
  name: string,
  projects: readonly { slug: string; info: { name: string } }[] | undefined,
  operations: readonly { name: string; operation: string; finished?: number | null }[] | undefined,
  /** The project being renamed: its own name is free. */
  except?: string,
): boolean {
  const key = name.trim().toLowerCase();
  if (key === "") return false;
  return (
    (projects ?? []).some((p) => p.slug !== except && p.info.name.trim().toLowerCase() === key) ||
    (operations ?? []).some((o) => (o.finished ?? null) === null && o.operation !== "project-remove" && o.name.trim().toLowerCase() === key)
  );
}
