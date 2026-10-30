// Analysis › Optimization (AM5 module 7): the facilities whose catchments
// cover the most population, optionally with a minimum per zone.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";
import { fileOf, tagsOf } from "./catalogue";
import { type FacilityInput, type FacilityMessage, NO_FACILITIES, toFacilitySource, updateFacilities } from "./facility-selection";
import { NO_ZONES, toZoneLayer, type ZoneInput, type ZoneMessage, updateZones, zoneIssues } from "./zones";

export const ANALYSIS = "optimization";

export interface OptimizationDraft {
  /** Catchment polygons; the identifier field holds each catchment's facility. */
  catchments: FacilityInput;
  population: number | null;
  /** Total number of facilities to select, as typed. */
  count: string;
  /** AM5 "Ensure minimum facilities per admin unit". */
  adminCheck: boolean;
  facilities: FacilityInput;
  zones: ZoneInput;
  /** Minimum facilities per zone, as typed. */
  perZone: string;
  tags: string;
}

// AM5 defaults: 10 facilities, 1 per zone.
export const EMPTY: OptimizationDraft = {
  catchments: NO_FACILITIES,
  population: null,
  count: "10",
  adminCheck: false,
  facilities: NO_FACILITIES,
  zones: NO_ZONES,
  perZone: "1",
  tags: "",
};

export type OptimizationMessage =
  | { type: "catchments"; message: FacilityMessage }
  | { type: "facilities"; message: FacilityMessage }
  | { type: "zones"; message: ZoneMessage }
  | { type: "population"; id: number | null }
  | { type: "count"; text: string }
  | { type: "adminCheck"; on: boolean }
  | { type: "perZone"; text: string }
  | { type: "tags"; text: string };

export function updateOptimization(draft: OptimizationDraft, message: OptimizationMessage): OptimizationDraft {
  switch (message.type) {
    case "catchments":
    case "facilities": {
      const input = updateFacilities(draft[message.type], message.message);
      return input === draft[message.type] ? draft : { ...draft, [message.type]: input };
    }
    case "zones": {
      const zones = updateZones(draft.zones, message.message);
      return zones === draft.zones ? draft : { ...draft, zones };
    }
    case "population":
      return { ...draft, population: message.id };
    case "count":
      return { ...draft, count: message.text };
    case "adminCheck":
      return { ...draft, adminCheck: message.on };
    case "perZone":
      return { ...draft, perZone: message.text };
    case "tags":
      return { ...draft, tags: message.text };
  }
}

/** A whole number above 0, or `undefined`. */
export function positiveInteger(text: string): number | undefined {
  const n = Number(text.trim());
  return text.trim() !== "" && Number.isSafeInteger(n) && n > 0 ? n : undefined;
}

export interface Built {
  document: Omit<Jobs.OptimizationJob, "output_dir"> | null;
  issues: Key[];
  tags: string[];
}

export function buildJob(draft: OptimizationDraft, context: { datasets: ReadonlyMap<number, Api.Dataset> }): Built {
  const catchments = fileOf(context, draft.catchments.layer);
  const population = fileOf(context, draft.population);
  const facilities = fileOf(context, draft.facilities.layer);
  const zones = fileOf(context, draft.zones.layer);
  const count = positiveInteger(draft.count);
  const perZone = positiveInteger(draft.perZone);
  const { tags, issues: tagIssues } = tagsOf(draft.tags);
  const issues: Key[] = [
    ...(catchments === undefined ? ["validation_m7_no_catchment" as const] : []),
    ...(catchments !== undefined && draft.catchments.idField === "" ? ["validation_m7_no_catchment_id_field" as const] : []),
    ...(population === undefined ? ["validation_m7_no_population" as const] : []),
    ...(count === undefined ? ["validation_m7_invalid_n_tot" as const] : []),
    ...tagIssues,
  ];
  if (draft.adminCheck) {
    if (facilities === undefined) issues.push("validation_m7_no_hf_for_admin_check");
    else if (draft.facilities.idField === "") issues.push("validation_m7_no_hf_id_field_for_admin_check");
    issues.push(...zoneIssues(draft.zones, zones, "validation_m7_no_zone_for_admin_check"));
    if (perZone === undefined) issues.push("validation_m7_invalid_np_admin");
  }
  if (issues.length > 0 || catchments === undefined || population === undefined || count === undefined) {
    return { document: null, issues, tags };
  }
  const quota =
    draft.adminCheck && facilities !== undefined && zones !== undefined && perZone !== undefined
      ? {
          facilities: toFacilitySource(facilities, draft.facilities.idField, draft.facilities.selection),
          zones: toZoneLayer(zones, draft.zones),
          min_per_zone: perZone,
        }
      : null;
  return {
    document: {
      schema_version: 1,
      catchments: toFacilitySource(catchments, draft.catchments.idField, draft.catchments.selection),
      population,
      max_facilities: count,
      zone_quota: quota,
    },
    issues,
    tags,
  };
}
