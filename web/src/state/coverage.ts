// Analysis › Geographic coverage (AM5 module 3): facilities, in a processing
// order, take the population they reach up to their capacity.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import { fileOf, pickField, tagsOf } from "./catalogue";
import { type FacilityInput, type FacilityMessage, facilityIssues, NO_FACILITIES, toFacilitySource, updateFacilities } from "./facility-selection";
import { type Context, isTravelMessage, limitSeconds, TRAVEL, type TravelDraft, type TravelMessage, travelJob, updateTravel } from "./travel";
import { NO_ZONES, toZoneLayer, type ZoneInput, type ZoneMessage, updateZones, zoneIssues } from "./zones";

export const ANALYSIS = "geographic-coverage";

/** AM5 limit when the maximum travel time is 0 ("unlimited"): 32 767 minutes. */
export const UNLIMITED_S = 32_767 * 60;

/** Fields of the facility layer the analysis reads. */
export type CoverageField = "nameField" | "capacityField" | "orderField";

/** Options (AM5 `mod3param`), all but the zonal one written as job flags. */
export type CoverageOption = "catchments" | "removeCovered" | "populationOnBarriers" | "zonal" | "ignoreCapacity" | "descending";

export interface CoverageDraft extends TravelDraft, Record<CoverageOption, boolean> {
  population: number | null;
  facilities: FacilityInput;
  nameField: string;
  capacityField: string;
  /** AM5 "table order" (a field) or "travel time" (population reached first). */
  order: "field" | "population";
  orderField: string;
  /** Travel time of the "travel time" order, minutes as typed. */
  orderMinutes: string;
  zones: ZoneInput;
}

export const EMPTY: CoverageDraft = {
  ...TRAVEL,
  population: null,
  facilities: NO_FACILITIES,
  nameField: "",
  capacityField: "",
  order: "field",
  orderField: "",
  orderMinutes: "120",
  // AM5 defaults: descending order; catchments, removal and population on barriers checked.
  descending: true,
  catchments: true,
  removeCovered: true,
  populationOnBarriers: true,
  zonal: false,
  ignoreCapacity: false,
  zones: NO_ZONES,
};

export type CoverageMessage =
  | TravelMessage
  | { type: "population"; id: number | null }
  | { type: "facilities"; message: FacilityMessage }
  | { type: "field"; field: CoverageField; value: string }
  | { type: "order"; order: CoverageDraft["order"] }
  | { type: "orderMinutes"; text: string }
  | { type: "option"; option: CoverageOption; on: boolean }
  | { type: "zones"; message: ZoneMessage };

export function updateCoverage(draft: CoverageDraft, message: CoverageMessage): CoverageDraft {
  if (isTravelMessage(message)) return updateTravel(draft, message);
  switch (message.type) {
    case "population":
      return { ...draft, population: message.id };
    case "facilities": {
      const facilities = updateFacilities(draft.facilities, message.message);
      if (facilities === draft.facilities) return draft;
      // The fields belong to the layer.
      const layerChanged = facilities.layer !== draft.facilities.layer;
      return layerChanged ? { ...draft, facilities, nameField: "", capacityField: "", orderField: "" } : { ...draft, facilities };
    }
    case "field":
      return { ...draft, [message.field]: message.value };
    case "order":
      return { ...draft, order: message.order };
    case "orderMinutes":
      return { ...draft, orderMinutes: message.text };
    case "option":
      return { ...draft, [message.option]: message.on };
    case "zones": {
      const zones = updateZones(draft.zones, message.message);
      return zones === draft.zones ? draft : { ...draft, zones };
    }
  }
}

/** The draft as shown: name, capacity and order fields default to `name` and `capacity` when present. */
export function withFieldDefaults(draft: CoverageDraft, fields: readonly string[]): CoverageDraft {
  const nameField = pickField(draft.nameField, fields, ["name"]);
  const capacityField = pickField(draft.capacityField, fields, ["capacity"]);
  const orderField = pickField(draft.orderField, fields, ["capacity"]);
  if (nameField === draft.nameField && capacityField === draft.capacityField && orderField === draft.orderField) return draft;
  return { ...draft, nameField, capacityField, orderField };
}

export interface Built {
  document: Omit<Jobs.GeographicCoverageJob, "output_dir"> | null;
  issues: Key[];
  /** AM5 information messages (not blocking). */
  notices: Key[];
  tags: string[];
}

export function buildJob(draft: CoverageDraft, context: Context): Built {
  const travel = travelJob(draft, context);
  const population = fileOf(context, draft.population);
  const facilities = fileOf(context, draft.facilities.layer);
  const zones = fileOf(context, draft.zones.layer);
  const limit = limitSeconds(draft.minutes);
  const orderLimit = limitSeconds(draft.orderMinutes);
  const { tags, issues: tagIssues } = tagsOf(draft.tags);
  const issues: Key[] = [
    ...travel.landCoverIssues,
    ...facilityIssues(draft.facilities, facilities),
    ...(limit === undefined ? ["srv_analysis_accessibility_max_travel_time_input" as const] : []),
    ...tagIssues,
    ...travel.scenarioIssues,
    ...(population === undefined ? ["srv_analysis_accessibility_select_population" as const] : []),
    ...(facilities !== undefined && !draft.ignoreCapacity && draft.capacityField === "" ? ["srv_analysis_accessibility_set_capacity_warning" as const] : []),
    ...(draft.order === "population" && (orderLimit === undefined || orderLimit === null) ? ["analysis.order_travel_time" as const] : []),
    ...(draft.order === "field" && facilities !== undefined && draft.orderField === "" ? ["analysis.order_field_missing" as const] : []),
    ...(draft.zonal ? zoneIssues(draft.zones, zones) : []),
  ];
  const notices: Key[] = [
    ...(limit === null ? ["srv_analysis_accessibility_max_travel_time_set_0min" as const] : []),
    ...(draft.populationOnBarriers ? [] : ["srv_analysis_accessibility_no_pop_barrier_warning" as const]),
    ...(!draft.removeCovered && draft.order !== "field" ? ["srv_analysis_accessibility_facilities_processing_order" as const] : []),
  ];
  if (issues.length > 0 || travel.fields === undefined || population === undefined || facilities === undefined || limit === undefined) {
    return { document: null, issues, notices, tags };
  }
  const order: Jobs.CoverageOrder =
    draft.order === "field"
      ? { field: { field: draft.orderField, descending: draft.descending } }
      : { population: { limit_s: orderLimit ?? 0, descending: draft.descending } };
  return {
    document: {
      schema_version: 1,
      ...travel.fields,
      population,
      facilities: toFacilitySource(facilities, draft.facilities.idField, draft.facilities.selection),
      name_field: draft.nameField === "" ? null : draft.nameField,
      capacity_field: draft.ignoreCapacity ? null : draft.capacityField,
      direction: draft.direction,
      limit_s: limit ?? UNLIMITED_S,
      band_s: 60,
      order,
      catchments: draft.catchments,
      remove_covered: draft.removeCovered,
      population_on_barriers: draft.populationOnBarriers,
      ignore_capacity: draft.ignoreCapacity,
      zones: draft.zonal && zones !== undefined ? toZoneLayer(zones, draft.zones) : null,
    },
    issues,
    notices,
    tags,
  };
}
