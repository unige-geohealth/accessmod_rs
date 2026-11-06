// The Data screen's document: the rows selected in "Available data" and the
// tags edited in the table (Data › Rename) until they are committed.
// Changed only by messages through the pure `updateData`.
import type { Api } from "../generated/types";
import { datasetIds, normalizeTags } from "./catalogue";

export interface DataDocument {
  /**
   * Rows the user selected (`d<id>`, `j<run>`, in selection order) under
   * the run filter `run`. Under another run filter (a new filter, Back,
   * Forward), or before any selection, the selection is every row of the
   * run filter, none without one ("Show in data", "Filter last results").
   */
  selected: { run: number | null; keys: readonly string[] } | "run";
  /** Tags typed in the table, by dataset, not yet committed. */
  edits: Readonly<Record<number, string>>;
}

export type DataMessage =
  /** The user selected `keys` while the run filter was `run`. */
  | { type: "select"; run: number | null; keys: string[] }
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
      return doc.selected !== "run" && doc.selected.run === message.run && same(doc.selected.keys, message.keys)
        ? doc
        : { ...doc, selected: { run: message.run, keys: [...message.keys] } };
    case "editTags":
      return doc.edits[message.id] === message.text ? doc : { ...doc, edits: { ...doc.edits, [message.id]: message.text } };
    case "discardEdits":
      return Object.keys(doc.edits).length === 0 ? doc : { ...doc, edits: {} };
    case "removed": {
      const gone = new Set(message.ids);
      const edits = Object.fromEntries(Object.entries(doc.edits).filter(([id]) => !gone.has(Number(id))));
      if (doc.selected === "run") return { ...doc, edits };
      const keys = doc.selected.keys.filter((key) => !datasetIds([key]).some((id) => gone.has(id)));
      return { selected: same(keys, doc.selected.keys) ? doc.selected : { ...doc.selected, keys }, edits };
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

/**
 * Keys of the selected rows among `visible`, the rows the filter shows
 * under run filter `run`: a hidden row is never selected (Delete acts on
 * what is seen).
 */
export function selectedKeys(doc: DataDocument, visible: readonly string[], run: number | null): readonly string[] {
  if (doc.selected === "run" || doc.selected.run !== run) return run === null ? [] : visible;
  const shown = new Set(visible);
  return doc.selected.keys.filter((key) => shown.has(key));
}

/** Selected datasets that still exist, in catalogue order. */
export function selectedDatasets(keys: readonly string[], datasets: readonly Api.Dataset[]): Api.Dataset[] {
  const ids = new Set(datasetIds(keys));
  return datasets.filter((d) => ids.has(d.id));
}
