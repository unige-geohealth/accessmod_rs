// The Data screen's document: the rows selected in "Available data" and the
// tags edited in the table (Data › Rename) until they are committed.
// Changed only by messages through the pure `updateData`.
import type { Api } from "../generated/types";
import { datasetIds, normalizeTags } from "./catalogue";

export interface DataDocument {
  /**
   * Keys of the selected rows (`d<id>`, `j<run>`), in selection order;
   * `"run"` until the user selects: every row of the run filter, none
   * without one ("Show in data", "Filter last results").
   */
  selected: readonly string[] | "run";
  /** Tags typed in the table, by dataset, not yet committed. */
  edits: Readonly<Record<number, string>>;
}

export type DataMessage =
  | { type: "select"; keys: string[] }
  /** The filter changed: the selection follows it again. */
  | { type: "filtered" }
  | { type: "editTags"; id: number; text: string }
  | { type: "discardEdits" }
  /** These datasets were removed on the server. */
  | { type: "removed"; ids: number[] }
  /** These edits were committed; an edit typed since then stays. */
  | { type: "renamed"; changes: TagChange[] };

export const EMPTY: DataDocument = { selected: "run", edits: {} };

const same = <T>(a: readonly T[], b: readonly T[]) => a.length === b.length && a.every((x, i) => x === b[i]);

export function updateData(doc: DataDocument, message: DataMessage): DataDocument {
  switch (message.type) {
    case "select":
      return doc.selected !== "run" && same(doc.selected, message.keys) ? doc : { ...doc, selected: [...message.keys] };
    case "filtered":
      return doc.selected === "run" ? doc : { ...doc, selected: "run" };
    case "editTags":
      return doc.edits[message.id] === message.text ? doc : { ...doc, edits: { ...doc.edits, [message.id]: message.text } };
    case "discardEdits":
      return Object.keys(doc.edits).length === 0 ? doc : { ...doc, edits: {} };
    case "removed": {
      const gone = new Set(message.ids);
      const edits = Object.fromEntries(Object.entries(doc.edits).filter(([id]) => !gone.has(Number(id))));
      if (doc.selected === "run") return { ...doc, edits };
      const selected = doc.selected.filter((key) => !datasetIds([key]).some((id) => gone.has(id)));
      return { selected: same(selected, doc.selected) ? doc.selected : selected, edits };
    }
    case "renamed": {
      const committed = new Map(message.changes.map((c) => [c.id, c.text]));
      const edits = Object.fromEntries(Object.entries(doc.edits).filter(([id, text]) => committed.get(Number(id)) !== text));
      return { ...doc, edits };
    }
  }
}

/** A tag change to commit with `dataset.tag`. */
export interface TagChange {
  id: number;
  /** The edit as typed, to tell it from a later one. */
  text: string;
  tags: string[];
}

/** Edits that change a dataset's tags (normalised), in identifier order. */
export function tagChanges(doc: DataDocument, datasets: readonly Api.Dataset[]): TagChange[] {
  const out: TagChange[] = [];
  for (const d of datasets) {
    const text = doc.edits[d.id];
    if (text === undefined) continue;
    const tags = normalizeTags(text);
    if (tags.join(" ") !== d.tags.join(" ")) out.push({ id: d.id, text, tags });
  }
  return out;
}

/** Keys of the selected rows; `visible`: the keys of the rows the filter shows. */
export function selectedKeys(doc: DataDocument, visible: readonly string[], runFilter: boolean): readonly string[] {
  if (doc.selected !== "run") return doc.selected;
  return runFilter ? visible : [];
}

/** Selected datasets that still exist, in catalogue order. */
export function selectedDatasets(keys: readonly string[], datasets: readonly Api.Dataset[]): Api.Dataset[] {
  const ids = new Set(datasetIds(keys));
  return datasets.filter((d) => ids.has(d.id));
}
