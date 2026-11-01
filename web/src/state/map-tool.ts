// Toolbox › Interactive map (AM5 `amGisPreview`): the raster shown, its
// opacity, the point queried, the facilities shown with a label, and the
// facilities moved (history with undo and redo) until saved as a new
// facilities dataset. Positions are WGS 84, as the map gives them.
import type { Key } from "@accessmod/ui/i18n";
import { normalizeTags, pickField } from "./catalogue";

export interface Move {
  fid: number;
  lon: number;
  lat: number;
}

export interface MapDraft {
  raster: number | null;
  /** 0 to 1. */
  opacity: number;
  /** Last point clicked, to query the raster. */
  clicked: { lon: number; lat: number } | null;
  facilities: number | null;
  labelField: string;
  editing: boolean;
  /** Moves applied, oldest first, and moves undone (latest last). */
  history: readonly Move[];
  undone: readonly Move[];
  tags: string;
}

export const EMPTY: MapDraft = {
  raster: null,
  opacity: 0.8,
  clicked: null,
  facilities: null,
  labelField: "",
  editing: false,
  history: [],
  undone: [],
  tags: "",
};

export type MapMessage =
  | { type: "raster"; id: number | null }
  | { type: "opacity"; value: number }
  | { type: "click"; lon: number; lat: number }
  | { type: "facilities"; id: number | null }
  | { type: "labelField"; field: string }
  | { type: "editing"; on: boolean }
  /** A move on facilities dataset `layer` (the layer shown, pinned by its first move). */
  | { type: "move"; layer: number; move: Move }
  | { type: "undo" }
  | { type: "redo" }
  /** Save of the first `count` moves of `layer` succeeded. */
  | { type: "saved"; layer: number; count: number }
  | { type: "tags"; text: string };

export function updateMap(draft: MapDraft, message: MapMessage): MapDraft {
  switch (message.type) {
    case "raster":
      return { ...draft, raster: message.id, clicked: null };
    case "opacity":
      return { ...draft, opacity: Math.min(1, Math.max(0, message.value)) };
    case "click":
      return { ...draft, clicked: { lon: message.lon, lat: message.lat } };
    case "facilities":
      // Moves belong to a layer.
      return message.id === draft.facilities ? draft : { ...draft, facilities: message.id, labelField: "", history: [], undone: [] };
    case "labelField":
      return { ...draft, labelField: message.field };
    case "editing":
      return { ...draft, editing: message.on };
    case "move":
      // Moves of another layer (the default changed) are not kept.
      return draft.facilities === message.layer
        ? { ...draft, history: [...draft.history, message.move], undone: [] }
        : { ...draft, facilities: message.layer, history: [message.move], undone: [] };
    case "undo": {
      const last = draft.history.at(-1);
      return last === undefined ? draft : { ...draft, history: draft.history.slice(0, -1), undone: [...draft.undone, last] };
    }
    case "redo": {
      const next = draft.undone.at(-1);
      return next === undefined ? draft : { ...draft, history: [...draft.history, next], undone: draft.undone.slice(0, -1) };
    }
    case "saved":
      // Moves made while saving stay; another layer since: nothing to clear.
      if (draft.facilities !== message.layer) return draft;
      return { ...draft, history: draft.history.slice(message.count), undone: [], tags: draft.history.length > message.count ? draft.tags : "" };
    case "tags":
      return { ...draft, tags: message.text };
  }
}

/** The draft as shown: the first raster and facilities, a name-like label. */
export function withMapDefaults(
  draft: MapDraft,
  first: { raster: number | null; facilities: number | null },
  fieldsOf: (layer: number | null) => readonly string[],
): MapDraft {
  const raster = draft.raster ?? first.raster;
  const facilities = draft.facilities ?? first.facilities;
  const labelField = pickField(draft.labelField, fieldsOf(facilities), ["name", "label", "cat"]);
  if (raster === draft.raster && facilities === draft.facilities && labelField === draft.labelField) return draft;
  return { ...draft, raster, facilities, labelField };
}

/** Where each moved facility is now: its latest move. */
export function moves(draft: MapDraft): Move[] {
  const latest = new Map<number, Move>();
  for (const m of draft.history) latest.set(m.fid, m);
  return [...latest.values()].sort((a, b) => a.fid - b.fid);
}

/** AM5 checks before Save: a layer, a move, a tag. */
export function saveIssues(draft: MapDraft): Key[] {
  const issues: Key[] = [];
  if (draft.facilities === null) issues.push("tool_map_relocate_missing_layer");
  if (normalizeTags(draft.tags).length === 0) issues.push("tool_map_relocate_missing_tags");
  return issues;
}

/** A GeoJSON point feature collection with the moved facilities in place. */
export interface PointCollection {
  type: "FeatureCollection";
  features: { type: "Feature"; id?: number | string; geometry: { type: string; coordinates: unknown } | null; properties: Record<string, unknown> | null }[];
}

export function withMoves(collection: PointCollection, moved: readonly Move[]): PointCollection {
  if (moved.length === 0) return collection;
  const at = new Map(moved.map((m) => [m.fid, m]));
  return {
    ...collection,
    features: collection.features.map((f) => {
      const m = typeof f.id === "number" ? at.get(f.id) : undefined;
      return m === undefined ? f : { ...f, geometry: { type: "Point", coordinates: [m.lon, m.lat] } };
    }),
  };
}
