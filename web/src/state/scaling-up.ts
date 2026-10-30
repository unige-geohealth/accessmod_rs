// Analysis › Scaling up (AM5 module 6): new facilities placed one after the
// other on the most suitable cells, until a limit is reached.
import type { Jobs } from "../generated/jobs";
import { fileOf, tagsOf } from "./catalogue";
import type { Issue } from "./drafts";
import { type FacilityInput, type FacilityMessage, facilityIssues, NO_FACILITIES, toFacilitySource, updateFacilities } from "./facility-selection";
import {
  type CapacityRow,
  capacityChecks,
  DEFAULT_CAPACITIES,
  DEFAULT_EXCLUSIONS,
  DEFAULT_FACTORS,
  type ExclusionRow,
  type FactorRow,
  jobLayer,
  missingLayers,
  toExclusion,
  toFactor,
} from "./scaling-up-tables";
import { type Context, isTravelMessage, limitSeconds, TRAVEL, type TravelDraft, type TravelMessage, travelJob, updateTravel } from "./travel";

export const ANALYSIS = "scaling-up";

/** AM5's limit on new facilities when "unlimited" (0). */
export const UNLIMITED_FACILITIES = 1_000_000;

/** AM5 limit when the maximum travel time is 0 ("unlimited"): 32 767 minutes. */
const UNLIMITED_S = 32_767 * 60;

export type Table = "capacities" | "factors" | "exclusions";

export interface ScalingUpDraft extends TravelDraft {
  population: number | null;
  residual: number | null;
  /** AM5 "Start using selected existing facilities" (else "Start with empty layer"). */
  useExisting: boolean;
  facilities: FacilityInput;
  /** Selected AM5 tables, read into the rows until the user edits them. */
  capacityTable: number | null;
  factorTable: number | null;
  exclusionTable: number | null;
  /** Edited rows; `null`: the rows of the selected table (or AM5's initial rows). */
  capacities: readonly CapacityRow[] | null;
  factors: readonly FactorRow[] | null;
  exclusions: readonly ExclusionRow[] | null;
  /** Computation limits as typed; 0: none (AM5). */
  coverPercent: string;
  newFacilities: string;
  maxMinutes: string;
}

export const EMPTY: ScalingUpDraft = {
  ...TRAVEL,
  direction: "towards_facilities",
  population: null,
  residual: null,
  useExisting: true,
  facilities: NO_FACILITIES,
  capacityTable: null,
  factorTable: null,
  exclusionTable: null,
  capacities: null,
  factors: null,
  exclusions: null,
  coverPercent: "80",
  newFacilities: "0",
  maxMinutes: "0",
};

export type ScalingUpMessage =
  | TravelMessage
  | { type: "population" | "residual"; id: number | null }
  | { type: "useExisting"; on: boolean }
  | { type: "facilities"; message: FacilityMessage }
  | { type: "table"; table: Table; id: number | null }
  | { type: "capacities"; rows: readonly CapacityRow[] }
  | { type: "factors"; rows: readonly FactorRow[] }
  | { type: "exclusions"; rows: readonly ExclusionRow[] }
  | { type: "reset"; table: Table }
  | { type: "limit"; limit: "coverPercent" | "newFacilities" | "maxMinutes"; text: string };

const TABLE_FIELD = { capacities: "capacityTable", factors: "factorTable", exclusions: "exclusionTable" } as const;

export function updateScalingUp(draft: ScalingUpDraft, message: ScalingUpMessage): ScalingUpDraft {
  if (isTravelMessage(message)) return updateTravel(draft, message);
  switch (message.type) {
    case "population":
    case "residual":
      return { ...draft, [message.type]: message.id };
    case "useExisting":
      return { ...draft, useExisting: message.on };
    case "facilities": {
      const facilities = updateFacilities(draft.facilities, message.message);
      return facilities === draft.facilities ? draft : { ...draft, facilities };
    }
    // AM5: selecting a table loads its rows.
    case "table":
      return { ...draft, [TABLE_FIELD[message.table]]: message.id, [message.table]: null };
    case "capacities":
    case "factors":
    case "exclusions":
      return { ...draft, [message.type]: message.rows };
    case "reset":
      return { ...draft, [message.table]: null };
    case "limit":
      return { ...draft, [message.limit]: message.text };
  }
}

/** Rows read from the selected tables (`undefined`: none selected or not loaded). */
export interface TableRows {
  capacities?: readonly CapacityRow[];
  factors?: readonly FactorRow[];
  exclusions?: readonly ExclusionRow[];
}

/** The rows shown: edited, else read from the selected table, else AM5's initial rows. */
export function shownRows(draft: ScalingUpDraft, tables: TableRows) {
  return {
    capacities: draft.capacities ?? tables.capacities ?? DEFAULT_CAPACITIES,
    factors: draft.factors ?? tables.factors ?? DEFAULT_FACTORS,
    exclusions: draft.exclusions ?? tables.exclusions ?? DEFAULT_EXCLUSIONS,
  };
}

/** Reading of a selected AM5 table: still loading (or failed), or longer than one read. */
export type TableStatus = "pending" | "too_large";

/** Selected tables whose rows are not usable yet; edited rows no longer depend on their table. */
export function tableIssues(draft: ScalingUpDraft, status: Partial<Record<Table, TableStatus>>): Issue[] {
  return (["capacities", "factors", "exclusions"] as const).flatMap((table): Issue[] => {
    if (draft[table] !== null) return [];
    if (status[table] === "pending") return ["analysis.table_loading"];
    if (status[table] === "too_large") return ["analysis.table_too_large"];
    return [];
  });
}

/** A number above or equal to 0 as typed, or `undefined`. */
function nonNegative(text: string): number | undefined {
  const n = Number(text.trim());
  return text.trim() !== "" && Number.isFinite(n) && n >= 0 ? n : undefined;
}

export interface Built {
  document: Omit<Jobs.ScalingUpJob, "output_dir"> | null;
  issues: Issue[];
  notices: Issue[];
  tags: string[];
}

export function buildJob(
  draft: ScalingUpDraft,
  context: Context,
  rows: ReturnType<typeof shownRows>,
  status: Partial<Record<Table, TableStatus>> = {},
): Built {
  const travel = travelJob(draft, context);
  const population = fileOf(context, draft.population);
  const residual = fileOf(context, draft.residual);
  const facilities = draft.useExisting ? fileOf(context, draft.facilities.layer) : undefined;
  const limit = limitSeconds(draft.minutes);
  const cover = nonNegative(draft.coverPercent);
  const count = nonNegative(draft.newFacilities);
  const minutes = nonNegative(draft.maxMinutes);
  const { tags, issues: tagIssues } = tagsOf(draft.tags);
  const capacity = capacityChecks(rows.capacities);
  const missingFactors = missingLayers(rows.factors, context.datasets);
  const missingExclusions = missingLayers(rows.exclusions, context.datasets);
  const onlyFacilities = rows.factors.length > 0 && rows.factors.every((r) => r.layer === "facilities");
  const issues: Issue[] = [
    ...travel.landCoverIssues,
    ...(draft.useExisting ? facilityIssues(draft.facilities, facilities) : []),
    ...(limit === undefined ? ["srv_analysis_accessibility_max_travel_time_input" as const] : []),
    ...tagIssues,
    ...travel.scenarioIssues,
    ...(population === undefined ? ["srv_analysis_accessibility_select_population" as const] : []),
    ...tableIssues(draft, status),
    ...capacity.issues,
    ...(rows.factors.length === 0 ? ["srv_analysis_accessibility_suitability_table_missing_value" as const] : []),
    ...(!draft.useExisting && onlyFacilities ? ["srv_analysis_accessibility_suitability_table_add_non_dynamic_layer" as const] : []),
    ...(missingFactors.length > 0 ? [{ key: "srv_analysis_accessibility_suitability_table_missing_layer" as const, args: [missingFactors.join(", ")] }] : []),
    ...(missingExclusions.length > 0 ? [{ key: "srv_analysis_accessibility_exclusion_table_missing_layer" as const, args: [missingExclusions.join(", ")] }] : []),
    ...(cover === undefined || cover > 100 ? ["analysis.cover_percent_invalid" as const] : []),
    ...(count === undefined || !Number.isInteger(count) ? ["analysis.new_facilities_invalid" as const] : []),
    ...(minutes === undefined ? ["analysis.max_minutes_invalid" as const] : []),
  ];
  const unlimited = { cover: cover === 0, count: count === 0, minutes: minutes === 0 };
  const residualClass = draft.residual === null ? undefined : context.datasets.get(draft.residual)?.class;
  const notices: Issue[] = [
    ...(limit === null ? ["srv_analysis_accessibility_max_travel_time_set_0min" as const] : []),
    ...capacity.notices,
    ...(unlimited.cover && unlimited.count && unlimited.minutes
      ? ["srv_analysis_accessibility_scaling_up_unlimited" as const]
      : [
          ...(unlimited.cover ? ["srv_analysis_accessibility_coverage_100percent" as const] : []),
          ...(unlimited.minutes ? ["srv_analysis_accessibility_processing_unlimited" as const] : []),
          ...(unlimited.count ? ["srv_analysis_accessibility_new_facilities_unlimited" as const] : []),
        ]),
    ...(draft.useExisting ? ["srv_analysis_accessibility_selected_facilities_verification" as const] : []),
    ...(draft.useExisting && residualClass !== undefined && residualClass !== "rPopulationResidual"
      ? ["srv_analysis_accessibility_facilities_residual_pop_warning" as const]
      : []),
    ...(!draft.useExisting && residualClass === "rPopulationResidual" ? ["srv_analysis_accessibility_residual_pop_no_facilities" as const] : []),
  ];
  const factors = rows.factors.map((r) => [r, jobLayer(r.layer, context.datasets)] as const);
  const exclusions = rows.exclusions.map((r) => [r, jobLayer(r.layer, context.datasets)] as const);
  if (
    issues.length > 0 ||
    travel.fields === undefined ||
    population === undefined ||
    limit === undefined ||
    cover === undefined ||
    count === undefined ||
    minutes === undefined ||
    (draft.useExisting && facilities === undefined)
  ) {
    return { document: null, issues, notices, tags };
  }
  return {
    document: {
      schema_version: 1,
      ...travel.fields,
      population,
      population_residual: residual ?? null,
      facilities: facilities === undefined ? null : toFacilitySource(facilities, draft.facilities.idField, draft.facilities.selection),
      direction: draft.direction,
      limit_s: limit ?? UNLIMITED_S,
      band_s: 60,
      capacities: rows.capacities.map((r) => ({ min: r.min, max: r.max, label: r.label, capacity: r.capacity })),
      suitability: factors.flatMap(([r, layer]) => (layer === undefined ? [] : [toFactor(r, layer)])),
      exclusions: exclusions.flatMap(([r, layer]) => (layer === undefined ? [] : [toExclusion(r, layer)])),
      max_facilities: count === 0 ? UNLIMITED_FACILITIES : count,
      max_coverage_percent: cover === 0 ? 100 : cover,
      max_duration_s: minutes === 0 ? null : minutes * 60,
      catchments: true,
    },
    issues,
    notices,
    tags,
  };
}
