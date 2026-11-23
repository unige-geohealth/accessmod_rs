// Facilities an analysis uses (AM5 "Facility selection" table): every
// facility matching an attribute filter, or the ones checked. Checked ids go
// to the job as `ids`, so a job saved here replays headless on the same
// subset (docs/plan.md, travel-time job).
import type { Row } from "@accessmod/ui/am/data-table";
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

/** Every id once, in order of first appearance. */
function unique(ids: readonly unknown[]): Jobs.FacilityId[] {
  const seen = new Set<string>();
  const out: Jobs.FacilityId[] = [];
  for (const id of ids) {
    const key = idKey(id);
    if (!seen.has(key)) {
      seen.add(key);
      out.push(toFacilityId(id));
    }
  }
  return out;
}

/** `ids` as a selection; every facility checked is "all", so the job carries no `ids`. */
function asSelection(where: string, ids: Jobs.FacilityId[], everyId: readonly unknown[]): FacilitySelection {
  const keys = new Set(ids.map(idKey));
  const all = everyId.every((id) => keys.has(idKey(id)));
  return all ? { mode: "all", where } : { mode: "ids", where, ids };
}

/** AM5 [Select]: the `matching` facilities are checked too; `everyId` holds every one. */
export function selectMatching(selection: FacilitySelection, matching: readonly unknown[], everyId: readonly unknown[]): FacilitySelection {
  if (selection.mode === "all") return selection;
  return asSelection(selection.where, unique([...selection.ids, ...matching]), everyId);
}

/** AM5 [Unselect]: the `matching` facilities are unchecked. */
export function unselectMatching(selection: FacilitySelection, matching: readonly unknown[], everyId: readonly unknown[]): FacilitySelection {
  const out = new Set(matching.map(idKey));
  const base = selection.mode === "all" ? unique(everyId) : selection.ids;
  return asSelection(
    selection.where,
    base.filter((id) => !out.has(idKey(id))),
    everyId,
  );
}

/** What the AccessMod columns of a facility are computed from. */
export interface FacilityContext {
  /** A land cover is sampled (first sample). */
  landCover: boolean;
  /** Index of the population sample; absent: no `amPopCell` column; -1: no population chosen. */
  popAt?: number;
  /** Speed of each land cover class in the scenario (km/h). */
  speeds: ReadonlyMap<number, number>;
}

/**
 * A facility as shown (AM5 table): `amOnBarrier` (no land cover class at its
 * cell), `amOnZero` (its class moves at 0 km/h), `amCatLandCover`, and
 * `amPopCell` when asked, then the fields of the layer.
 */
export function facilityRow(
  values: readonly unknown[],
  columns: readonly string[],
  samples: readonly (number | null)[] | undefined,
  context: FacilityContext,
): Row {
  const cls = context.landCover ? (samples?.[0] ?? null) : null;
  const row: Row = {
    amOnBarrier: cls === null ? "yes" : "no",
    amOnZero: cls !== null && (context.speeds.get(cls) ?? 0) === 0 ? "yes" : "no",
    amCatLandCover: cls,
  };
  if (context.popAt !== undefined) row.amPopCell = context.popAt < 0 ? null : (samples?.[context.popAt] ?? 0);
  columns.forEach((c, k) => {
    const v = values[k];
    row[c] = v === null || v === undefined ? null : typeof v === "object" ? JSON.stringify(v) : (v as string | number | boolean);
  });
  return row;
}

/** The AccessMod columns of the table, before the fields of the layer. */
export const AM_COLUMNS = ["amOnBarrier", "amOnZero", "amCatLandCover"] as const;

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

/** The input as shown: AM5 preselects the first layer and its `cat` field (or the first of `prefer` present). */
export function withFacilityDefaults(
  input: FacilityInput,
  layer: number | null,
  fieldsOf: (layer: number | null) => readonly string[],
  prefer: readonly string[] = ["cat"],
): FacilityInput {
  const chosen = input.layer ?? layer;
  const fields = fieldsOf(chosen);
  const idField = pickField(input.idField, fields, prefer);
  return chosen === input.layer && idField === input.idField ? input : { ...input, layer: chosen, idField };
}

/** Missing layer, missing identifier field, empty selection (AM5 messages; `empty` for the last). */
export function facilityIssues(input: FacilityInput, path: string | undefined, empty: Key = "srv_analysis_accessibility_select_facilities"): Key[] {
  if (path === undefined) return ["srv_analysis_accessibility_missing_facility_layer"];
  const issues: Key[] = [];
  if (input.idField === "") issues.push("analysis.id_field_missing");
  if (input.selection.mode === "ids" && input.selection.ids.length === 0) issues.push(empty);
  return issues;
}
