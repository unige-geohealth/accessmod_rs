// Analysis › Zonal statistics (AM5 module 5): population covered within
// travel-time limits, per zone. No "Process finished" dialog: Update runs
// the analysis and the table of its output is shown in place.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";
import { fileOf } from "./catalogue";
import { NO_ZONES, toZoneLayer, type ZoneInput, type ZoneMessage, updateZones, zoneIssues } from "./zones";

export const ANALYSIS = "zonal-statistics";

export interface ZonalDraft {
  population: number | null;
  travelTime: number | null;
  zones: ZoneInput;
  /** Maximum travel times as typed, minutes, comma separated (AM5 default "0"). */
  minutes: string;
  /** AM5 "Format table to get individual columns for each time step". */
  wide: boolean;
}

export const EMPTY: ZonalDraft = { population: null, travelTime: null, zones: NO_ZONES, minutes: "0", wide: false };

export type ZonalMessage =
  | { type: "population" | "travelTime"; id: number | null }
  | { type: "zones"; message: ZoneMessage }
  | { type: "minutes"; text: string }
  | { type: "wide"; on: boolean };

export function updateZonal(draft: ZonalDraft, message: ZonalMessage): ZonalDraft {
  switch (message.type) {
    case "population":
    case "travelTime":
      return { ...draft, [message.type]: message.id };
    case "zones": {
      const zones = updateZones(draft.zones, message.message);
      return zones === draft.zones ? draft : { ...draft, zones };
    }
    case "minutes":
      return { ...draft, minutes: message.text };
    case "wide":
      return { ...draft, wide: message.on };
  }
}

/** Limits in seconds from "10, 20, 120" minutes: non-numeric, duplicated and non-positive values ignored (AM5). */
export function limitsOf(text: string): number[] {
  const minutes = text
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s !== "")
    .map(Number)
    .filter((n) => Number.isFinite(n) && n > 0);
  return [...new Set(minutes)].sort((a, b) => a - b).map((m) => m * 60);
}

export interface Built {
  document: Omit<Jobs.ZonalStatisticsJob, "output_dir"> | null;
  issues: Key[];
}

export function buildJob(draft: ZonalDraft, context: { datasets: ReadonlyMap<number, Api.Dataset>; dem: number | null }): Built {
  const population = fileOf(context, draft.population);
  const travelTime = fileOf(context, draft.travelTime);
  const zones = fileOf(context, draft.zones.layer);
  const dem = fileOf(context, context.dem);
  const limits = limitsOf(draft.minutes);
  const issues: Key[] = [
    ...(population === undefined ? ["srv_analysis_accessibility_select_population" as const] : []),
    ...(travelTime === undefined ? ["srv_analysis_accessibility_missing_travel_time" as const] : []),
    ...zoneIssues(draft.zones, zones, "srv_analysis_accessibility_missing_zone"),
    ...(limits.length === 0 ? ["analysis.time_limits_missing" as const] : []),
  ];
  if (issues.length > 0 || population === undefined || travelTime === undefined || zones === undefined || dem === undefined) {
    return { document: null, issues };
  }
  return {
    document: { schema_version: 1, dem, population, travel_time: travelTime, zones: toZoneLayer(zones, draft.zones), time_limits_s: limits },
    issues,
  };
}

/** The zonal output dataset of a run. */
export const zonalOutput = (record: Api.RunRecord | undefined, datasets: ReadonlyMap<number, Api.Dataset>): number | null =>
  record?.datasets.find((id) => datasets.get(id)?.class === "tZonalStat") ?? null;

type Cell = string | number | boolean | null;

/**
 * Rows of the output table, long (one row per zone and limit, as written) or
 * wide (AM5: one row per zone, covered population and percent per limit,
 * the limit in minutes in the column name).
 */
export function zonalTable(rows: Api.Rows | undefined, wide: boolean): { columns: string[]; data: Record<string, Cell>[] } {
  if (rows === undefined) return { columns: [], data: [] };
  const cell = (v: unknown): Cell => (v === undefined ? null : typeof v === "object" && v !== null ? JSON.stringify(v) : (v as Cell));
  const long = rows.rows.map((r) => Object.fromEntries(rows.columns.map((c, i) => [c, cell(r[i])])));
  if (!wide) return { columns: rows.columns, data: long };
  const limits = [...new Set(long.map((r) => Number(r.time_limit_s)))].sort((a, b) => a - b);
  const zones = new Map<string, Record<string, Cell>>();
  for (const r of long) {
    const key = String(r.zone_id);
    const zone = zones.get(key) ?? { zone_id: r.zone_id ?? null, label: r.label ?? null, population: r.population ?? null };
    const m = Number(r.time_limit_s) / 60;
    zone[`population_covered_${m}min`] = r.population_covered ?? null;
    zone[`coverage_percent_${m}min`] = r.coverage_percent ?? null;
    zones.set(key, zone);
  }
  const columns = ["zone_id", "label", "population", ...limits.flatMap((l) => [`population_covered_${l / 60}min`, `coverage_percent_${l / 60}min`])];
  return { columns, data: [...zones.values()] };
}
