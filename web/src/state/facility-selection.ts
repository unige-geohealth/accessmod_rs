// Facilities an analysis uses (AM5 "Facility selection" table): every
// facility matching an attribute filter, or the ones checked. Checked ids go
// to the job as `ids`, so a job saved here replays headless on the same
// subset (docs/plan.md, travel-time job).
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import { pickField } from "./catalogue";

export type FacilitySelection =
  /** Every facility matching `where` (AM5 default: all checked). */
  | { mode: "all"; where: string }
  /** Only `ids`, among the facilities matching `where`. */
  | { mode: "ids"; where: string; ids: readonly Jobs.FacilityId[] };

export const ALL: FacilitySelection = { mode: "all", where: "" };

/** Comparison key of an identifier, as the server's `id_key`: `4`, `4.0` and `"4"` match. */
export function idKey(value: unknown): string {
  return String(value ?? "");
}

/** Identifier for the job: integers stay numbers, everything else is text. */
export function toFacilityId(value: unknown): Jobs.FacilityId {
  return typeof value === "number" && Number.isSafeInteger(value) ? value : idKey(value);
}

export function isChecked(selection: FacilitySelection, id: unknown): boolean {
  if (selection.mode === "all") return true;
  const key = idKey(id);
  return selection.ids.some((i) => idKey(i) === key);
}

/** Checked ids of one page, as table row ids. */
export function checkedOnPage(selection: FacilitySelection, page: readonly unknown[]): string[] {
  if (selection.mode === "all") return page.map(idKey);
  const keys = new Set(selection.ids.map(idKey));
  return page.map(idKey).filter((k) => keys.has(k));
}

/**
 * The selection after the user changed the checks of one page: `page` holds
 * the ids shown, `checked` those now checked. From "all", `everyId` (every id
 * matching the filter) is needed to know the others.
 */
export function checkPage(
  selection: FacilitySelection,
  page: readonly unknown[],
  checked: readonly unknown[],
  everyId?: readonly unknown[],
): FacilitySelection {
  const base = selection.mode === "all" ? (everyId ?? []).map(toFacilityId) : selection.ids;
  const shown = new Set(page.map(idKey));
  const kept = base.filter((id) => !shown.has(idKey(id)));
  const ids = [...kept, ...checked.map(toFacilityId)];
  return { mode: "ids", where: selection.where, ids };
}

export const count = (selection: FacilitySelection, matching: number): number =>
  selection.mode === "all" ? matching : selection.ids.length;

/** The job's facility source. */
export function toFacilitySource(path: string, idField: string, selection: FacilitySelection): Jobs.FacilitySource {
  const where = selection.where.trim();
  return {
    path,
    id_field: idField,
    ...(where === "" ? {} : { where }),
    ...(selection.mode === "ids" ? { ids: [...selection.ids] } : {}),
  };
}

/** A facility layer chosen in an analysis: the layer, its identifier field and the selection. */
export interface FacilityInput {
  layer: number | null;
  /** Identifier field; "" until chosen. */
  idField: string;
  selection: FacilitySelection;
}

export const NO_FACILITIES: FacilityInput = { layer: null, idField: "", selection: ALL };

export type FacilityMessage =
  | { type: "layer"; id: number | null }
  | { type: "idField"; field: string }
  | { type: "selection"; selection: FacilitySelection };

export function updateFacilities(input: FacilityInput, message: FacilityMessage): FacilityInput {
  switch (message.type) {
    case "layer":
      // Fields and identifiers belong to a layer.
      return message.id === input.layer ? input : { layer: message.id, idField: "", selection: ALL };
    case "idField":
      return message.field === input.idField ? input : { ...input, idField: message.field, selection: { mode: "all", where: input.selection.where } };
    case "selection":
      return { ...input, selection: message.selection };
  }
}

/** The input as shown: AM5 preselects the first layer and its `cat` field. */
export function withFacilityDefaults(input: FacilityInput, layer: number | null, fieldsOf: (layer: number | null) => readonly string[]): FacilityInput {
  const chosen = input.layer ?? layer;
  const fields = fieldsOf(chosen);
  const idField = pickField(input.idField, fields, ["cat"]);
  return chosen === input.layer && idField === input.idField ? input : { ...input, layer: chosen, idField };
}

/** Missing layer, missing identifier field, empty selection (AM5 messages). */
export function facilityIssues(input: FacilityInput, path: string | undefined): Key[] {
  if (path === undefined) return ["srv_analysis_accessibility_missing_facility_layer"];
  const issues: Key[] = [];
  if (input.idField === "") issues.push("analysis.id_field_missing");
  if (input.selection.mode === "ids" && input.selection.ids.length === 0) issues.push("srv_analysis_accessibility_select_facilities");
  return issues;
}
