// A zone layer chosen in an analysis (Geographic coverage, Zonal statistics,
// Optimization): the layer, its identifier and label fields.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import { pickField } from "./catalogue";

export interface ZoneInput {
  layer: number | null;
  /** Integer identifier field; "" until chosen. */
  idField: string;
  labelField: string;
}

export const NO_ZONES: ZoneInput = { layer: null, idField: "", labelField: "" };

export type ZoneMessage = { type: "layer"; id: number | null } | { type: "idField"; field: string } | { type: "labelField"; field: string };

export function updateZones(input: ZoneInput, message: ZoneMessage): ZoneInput {
  switch (message.type) {
    case "layer":
      return message.id === input.layer ? input : { layer: message.id, idField: "", labelField: "" };
    case "idField":
      return message.field === input.idField ? input : { ...input, idField: message.field };
    case "labelField":
      return message.field === input.labelField ? input : { ...input, labelField: message.field };
  }
}

/** The input as shown: the first layer, a `cat` identifier and a `name` label when present. */
export function withZoneDefaults(input: ZoneInput, layer: number | null, fieldsOf: (layer: number | null) => readonly string[]): ZoneInput {
  const chosen = input.layer ?? layer;
  const fields = fieldsOf(chosen);
  const idField = pickField(input.idField, fields, ["cat", "id"]);
  const labelField = pickField(input.labelField, fields, ["name", "label"]);
  if (chosen === input.layer && idField === input.idField && labelField === input.labelField) return input;
  return { layer: chosen, idField, labelField };
}

/** AM5 zone messages; `missing` when no layer is chosen. */
export function zoneIssues(input: ZoneInput, path: string | undefined, missing: Key = "srv_analysis_accessibility_select_zone_warning"): Key[] {
  if (path === undefined) return [missing];
  const issues: Key[] = [];
  if (input.idField === "") issues.push("srv_analysis_accessibility_zonal_id_missing");
  if (input.labelField === "") issues.push("srv_analysis_accessibility_zonal_label_missing");
  return issues;
}

export const toZoneLayer = (path: string, input: ZoneInput): Jobs.ZoneLayer => ({ path, id_field: input.idField, label_field: input.labelField });
