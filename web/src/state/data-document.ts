// The Data screen's document: the rows selected in "Available data" and the
// tags edited in the table (Data › Rename) until they are committed.
// Changed only by messages through the pure `updateData`.
import type { Api } from "../generated/types";
import { normalizeTags } from "./catalogue";

export interface DataDocument {
  /** Selected dataset identifiers, in selection order. */
  selected: readonly number[];
  /** Tags typed in the table, by dataset, not yet committed. */
  edits: Readonly<Record<number, string>>;
}

export type DataMessage =
  | { type: "select"; ids: number[] }
  | { type: "editTags"; id: number; text: string }
  | { type: "discardEdits" }
  /** These datasets were removed on the server. */
  | { type: "removed"; ids: number[] }
  /** These edits were committed; an edit typed since then stays. */
  | { type: "renamed"; changes: TagChange[] };

export const EMPTY: DataDocument = { selected: [], edits: {} };

const same = (a: readonly number[], b: readonly number[]) => a.length === b.length && a.every((x, i) => x === b[i]);

export function updateData(doc: DataDocument, message: DataMessage): DataDocument {
  switch (message.type) {
    case "select":
      return same(doc.selected, message.ids) ? doc : { ...doc, selected: [...message.ids] };
    case "editTags":
      return doc.edits[message.id] === message.text ? doc : { ...doc, edits: { ...doc.edits, [message.id]: message.text } };
    case "discardEdits":
      return Object.keys(doc.edits).length === 0 ? doc : { ...doc, edits: {} };
    case "removed": {
      const gone = new Set(message.ids);
      const edits = Object.fromEntries(Object.entries(doc.edits).filter(([id]) => !gone.has(Number(id))));
      const selected = doc.selected.filter((id) => !gone.has(id));
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

/** Selected datasets that still exist, in catalogue order. */
export function selectedDatasets(doc: DataDocument, datasets: readonly Api.Dataset[]): Api.Dataset[] {
  const ids = new Set(doc.selected);
  return datasets.filter((d) => ids.has(d.id));
}
