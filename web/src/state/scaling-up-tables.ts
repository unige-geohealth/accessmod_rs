// The three tables of Analysis › Scaling up, as AM5 shows them: capacities
// of new facilities (editable), suitability factors and exclusion areas
// (added from forms, removed by selection). Rows read from AM5 tables
// (`tCapacity`, `tSuitability`, `tExclusion`) name layers as AM5 does
// (`vRoad__demo`, `vOutputFacility`); the job takes dataset paths.
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";
import { kindOf } from "./catalogue";
import type { Issue } from "./drafts";

/** A layer of a rule: the dynamic facilities or population, a dataset, or an AM5 name matching none. */
export type LayerRef = "facilities" | "population" | { dataset: number } | { missing: string };

/** AM5 names of the dynamic layers. */
export const DYNAMIC = { facilities: "vOutputFacility", population: "rOutputPopulation" } as const;

export interface CapacityRow {
  min: number;
  max: number;
  label: string;
  capacity: number;
}

/** AM5 factor names (`popsum`, `dist`, `traveltime`, `priority`). */
export const FACTORS = ["popsum", "dist", "traveltime", "priority"] as const;
export type FactorKind = (typeof FACTORS)[number];

export interface FactorRow {
  id: number;
  factor: FactorKind;
  layer: LayerRef;
  weight: number;
  /** AM5 `p`: higher values more (`hvms`) or less (`hvls`) suitable. */
  prefer: "hvms" | "hvls";
  /** AM5 `r` (popsum), km. */
  radiusKm: number;
  /** AM5 `t`, `d`, `k` (traveltime). */
  method: "iso" | "aniso";
  direction: "from" | "to";
  knight: boolean;
}

export interface ExclusionRow {
  id: number;
  layer: LayerRef;
  bufferKm: number;
  method: "keep_inside" | "keep_outside";
}

/** AM5 initial rows when no table is selected. */
export const DEFAULT_CAPACITIES: readonly CapacityRow[] = [{ min: 0, max: 0, label: "label", capacity: 0 }];
export const NEW_FACTOR: Omit<FactorRow, "id" | "layer"> = {
  factor: "popsum",
  weight: 1,
  prefer: "hvms",
  radiusKm: 1,
  method: "iso",
  direction: "to",
  knight: false,
};
export const DEFAULT_FACTORS: readonly FactorRow[] = [{ id: 1, layer: "population", ...NEW_FACTOR }];
export const DEFAULT_EXCLUSIONS: readonly ExclusionRow[] = [{ id: 1, layer: "facilities", bufferKm: 5, method: "keep_inside" }];

/** Next row identifier. */
export const nextId = (rows: readonly { id: number }[]): number => rows.reduce((m, r) => Math.max(m, r.id), 0) + 1;

/** Values of the column `name` of a table page. */
function column(rows: Api.Rows, name: string): unknown[] {
  const at = rows.columns.indexOf(name);
  return rows.rows.map((r) => (at < 0 ? undefined : r[at]));
}

const number = (v: unknown): number => (typeof v === "number" ? v : v === null || v === undefined || v === "" ? Number.NaN : Number(v));

/** Capacity rows of an AM5 capacity table. */
export function capacityRows(rows: Api.Rows): CapacityRow[] {
  const [min, max, label, capacity] = ["min", "max", "label", "capacity"].map((c) => column(rows, c));
  return rows.rows.map((_, i) => ({ min: number(min?.[i]), max: number(max?.[i]), label: String(label?.[i] ?? ""), capacity: number(capacity?.[i]) }));
}

/** The dataset an AM5 layer name (`class__tag_tag`) refers to: same class, same tags. */
export function layerOf(name: string, datasets: readonly Api.Dataset[]): LayerRef {
  if (name === DYNAMIC.facilities) return "facilities";
  if (name === DYNAMIC.population) return "population";
  const [cls, tags = ""] = name.split("@")[0]!.split("__");
  const wanted = tags.split("_").filter((t) => t !== "").sort().join(" ");
  const found = datasets.filter((d) => d.class === cls && [...d.tags].sort().join(" ") === wanted).sort((a, b) => b.id - a.id)[0];
  return found ? { dataset: found.id } : { missing: name };
}

/** AM5 options `r=1;p=hvms;t=aniso;d=to;k=n16`. */
function parseOptions(text: string): Record<string, string> {
  return Object.fromEntries(
    text
      .split(";")
      .map((o) => o.split("="))
      .filter((kv): kv is [string, string] => kv.length === 2)
      .map(([k, v]) => [k.trim(), v.trim()]),
  );
}

export function factorOptions(row: FactorRow): string {
  const options: string[] = [];
  if (row.factor === "popsum") options.push(`r=${row.radiusKm}`);
  if (row.factor === "traveltime") {
    if (row.method === "aniso") options.push(`d=${row.direction}`);
    options.push(`t=${row.method}`, `k=${row.knight ? "n16" : "n8"}`);
  }
  options.push(`p=${row.prefer}`);
  return options.join(";");
}

/** Suitability rows of an AM5 suitability table. */
export function factorRows(rows: Api.Rows, datasets: readonly Api.Dataset[]): FactorRow[] {
  const [factor, layer, weight, options] = ["factor", "layer", "weight", "options"].map((c) => column(rows, c));
  return rows.rows.flatMap((_, i) => {
    const kind = String(factor?.[i] ?? "");
    if (!(FACTORS as readonly string[]).includes(kind)) return [];
    const o = parseOptions(String(options?.[i] ?? ""));
    return [
      {
        id: i + 1,
        factor: kind as FactorKind,
        layer: layerOf(String(layer?.[i] ?? ""), datasets),
        weight: number(weight?.[i]),
        prefer: o.p === "hvls" ? "hvls" : "hvms",
        radiusKm: o.r === undefined ? NEW_FACTOR.radiusKm : Number(o.r),
        method: o.t === "aniso" ? "aniso" : "iso",
        direction: o.d === "from" ? "from" : "to",
        knight: o.k === "n16",
      },
    ];
  });
}

/** Exclusion rows of an AM5 exclusion table. */
export function exclusionRows(rows: Api.Rows, datasets: readonly Api.Dataset[]): ExclusionRow[] {
  const [layer, buffer, method] = ["layer", "buffer", "method"].map((c) => column(rows, c));
  return rows.rows.map((_, i) => ({
    id: i + 1,
    layer: layerOf(String(layer?.[i] ?? ""), datasets),
    bufferKm: number(buffer?.[i]),
    method: method?.[i] === "keep_outside" ? "keep_outside" : "keep_inside",
  }));
}

/** A rule layer in the job, or `undefined` when its dataset is missing. */
export function jobLayer(layer: LayerRef, datasets: ReadonlyMap<number, Api.Dataset>): Jobs.Layer | undefined {
  if (layer === "facilities" || layer === "population") return layer;
  if ("missing" in layer) return undefined;
  const d = datasets.get(layer.dataset);
  if (d === undefined) return undefined;
  return kindOf(d.class) === "raster" ? { raster: d.file } : { vector: { path: d.file } };
}

export function toFactor(row: FactorRow, layer: Jobs.Layer): Jobs.Factor {
  const common = { layer, weight: row.weight, prefer: row.prefer === "hvms" ? ("high" as const) : ("low" as const) };
  switch (row.factor) {
    case "popsum":
      return { factor: "population", ...common, window_m: row.radiusKm * 1000 };
    case "dist":
      return { factor: "distance", ...common };
    case "traveltime":
      return {
        factor: "travel_time",
        ...common,
        method: row.method === "aniso" ? "anisotropic" : "isotropic",
        direction: row.direction === "from" ? "from_facilities" : "towards_facilities",
        neighbourhood: row.knight ? 16 : 8,
      };
    case "priority":
      return { factor: "priority", ...common };
  }
}

export const toExclusion = (row: ExclusionRow, layer: Jobs.Layer): Jobs.Exclusion => ({
  layer,
  buffer_m: row.bufferKm * 1000,
  keep: row.method === "keep_inside" ? "inside" : "outside",
});

/** AM5 checks of the capacity table (`amValidateModule6CapacityTable`): errors, then information. */
export function capacityChecks(rows: readonly CapacityRow[]): { issues: Issue[]; notices: Issue[] } {
  const bad = (n: number) => !Number.isFinite(n);
  if (rows.length === 0 || rows.some((r) => bad(r.min) || bad(r.max) || bad(r.capacity) || r.label.trim() === "")) {
    return { issues: ["srv_analysis_accessibility_scaleup_table_missing_value"], notices: [] };
  }
  const issues: Issue[] = [];
  if (rows.some((r) => r.min >= r.max)) issues.push("srv_analysis_accessibility_scaleup_table_min_max_equality");
  if (rows[0]!.min !== 0) issues.push("srv_analysis_accessibility_scaleup_table_first_min_value_0");
  if (new Set(rows.map((r) => r.label)).size !== rows.length) issues.push("srv_analysis_accessibility_scaleup_capacity_duplicate_labels");
  for (let i = 1; i < rows.length; i++) {
    if (!(rows[i]!.capacity > rows[i - 1]!.capacity)) {
      issues.push("srv_analysis_accessibility_scaleup_capacity_not_incremental");
      break;
    }
    if (!(rows[i]!.min > rows[i - 1]!.max)) {
      issues.push("srv_analysis_accessibility_scaleup_min_value_greater_previous_max");
      break;
    }
  }
  const outside = rows.some((r) => r.capacity > r.max || r.capacity < r.min);
  return { issues, notices: outside ? ["srv_analysis_accessibility_scaleup_values_not_in_range"] : [] };
}

/** AM5 names of the layers missing from the project. */
export const missingLayers = (rows: readonly { layer: LayerRef }[], datasets: ReadonlyMap<number, Api.Dataset>): string[] =>
  rows.flatMap((r) => {
    if (typeof r.layer === "string") return [];
    if ("missing" in r.layer) return [r.layer.missing];
    return datasets.has(r.layer.dataset) ? [] : [`#${r.layer.dataset}`];
  });

/** Capacity rows after an edit of one cell (numbers from text, as the table gives them). */
export function editCapacity(rows: readonly CapacityRow[], index: number, field: string, value: unknown): CapacityRow[] {
  return rows.map((r, i) => {
    if (i !== index) return r;
    if (field === "label") return { ...r, label: String(value ?? "") };
    if (field === "min" || field === "max" || field === "capacity") return { ...r, [field]: number(value) };
    return r;
  });
}
