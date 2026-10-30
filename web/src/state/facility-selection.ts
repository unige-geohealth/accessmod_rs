// Facilities an analysis uses (AM5 "Facility selection" table): every
// facility matching an attribute filter, or the ones checked. Checked ids go
// to the job as `ids`, so a job saved here replays headless on the same
// subset (docs/plan.md, travel-time job).
import type { Jobs } from "../generated/jobs";

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
