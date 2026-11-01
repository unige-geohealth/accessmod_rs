// Toolbox › Interactive map (AM5 `amGisPreview`), a small GIS module:
// - Facilities relocator: a facilities layer over a reference raster; a
//   facility without a value there (barrier, NoData, off the grid) is to
//   inspect; Next goes to the next one; moves (history with undo and redo)
//   are saved as a new facilities dataset. Relocation stays manual: which
//   side of a one-cell barrier a facility is on takes context.
// - Additional layers: vector datasets of the project in a z-order.
// - Basemap: the offline schematic map, or a MapTiler style.
// Draw order, top first: facilities, additional layers, reference raster,
// basemap. Positions are WGS 84, as the map gives them.
import type { Key } from "@accessmod/ui/i18n";
import { normalizeTags, pickField } from "./catalogue";

export interface Move {
  fid: number;
  lon: number;
  lat: number;
}

/** Basemaps: the offline schematic map, or MapTiler styles (`maps/<id>/style.json`). */
export const BASEMAPS = ["schematic", "backdrop-v4", "dataviz-v4", "satellite-v4", "outdoor-v4"] as const;
export type Basemap = (typeof BASEMAPS)[number];

/** Additional layers shown at most. */
export const MAX_LAYERS = 10;

/** An additional layer: a vector dataset, shown or hidden. */
export interface MapLayer {
  id: number;
  visible: boolean;
}

export interface MapDraft {
  /** Reference raster of the relocator (usually the merged land cover). */
  reference: number | null;
  /** 0 to 1. */
  opacity: number;
  /** Last point clicked, to query the raster. */
  clicked: { lon: number; lat: number } | null;
  facilities: number | null;
  labelField: string;
  editing: boolean;
  /** A save is running: moves, undo, redo and the layer wait for its answer. */
  saving: boolean;
  /** Moves applied, oldest first, and moves undone (latest last). */
  history: readonly Move[];
  undone: readonly Move[];
  tags: string;
  /** Facility Next went to. */
  selected: number | null;
  /** Additional vector layers, top first; a hidden one keeps its place. */
  layers: readonly MapLayer[];
  basemap: Basemap;
  /** The online basemap failed to load: back to the schematic map, with a notice. */
  basemapFailed: boolean;
}

export const EMPTY: MapDraft = {
  reference: null,
  opacity: 0.8,
  clicked: null,
  facilities: null,
  labelField: "",
  editing: false,
  saving: false,
  history: [],
  undone: [],
  tags: "",
  selected: null,
  layers: [],
  basemap: "schematic",
  basemapFailed: false,
};

export type MapMessage =
  | { type: "reference"; id: number | null }
  | { type: "select"; fid: number | null }
  /** Added on top (at most `MAX_LAYERS`). */
  | { type: "addLayer"; id: number }
  | { type: "removeLayer"; id: number }
  | { type: "toggleLayer"; id: number }
  /** New order, top first. */
  | { type: "orderLayers"; ids: readonly number[] }
  | { type: "basemap"; basemap: Basemap }
  | { type: "basemapFailed" }
  | { type: "opacity"; value: number }
  | { type: "click"; lon: number; lat: number }
  | { type: "facilities"; id: number | null }
  | { type: "labelField"; field: string }
  | { type: "editing"; on: boolean }
  /** A move on facilities dataset `layer` (the layer shown, pinned by its first move). */
  | { type: "move"; layer: number; move: Move }
  | { type: "undo" }
  | { type: "redo" }
  | { type: "save" }
  /** The save wrote dataset `id`, a copy of the layer with the moves (same feature ids): it is shown and edited next. */
  | { type: "saved"; id: number }
  | { type: "saveFailed" }
  | { type: "tags"; text: string };

/** What a running save holds still. */
const HELD: ReadonlySet<MapMessage["type"]> = new Set(["facilities", "move", "undo", "redo", "save"]);

export function updateMap(draft: MapDraft, message: MapMessage): MapDraft {
  if (draft.saving && HELD.has(message.type)) return draft;
  switch (message.type) {
    case "reference":
      return { ...draft, reference: message.id, clicked: null };
    case "select":
      return { ...draft, selected: message.fid };
    case "addLayer":
      if (draft.layers.length >= MAX_LAYERS || draft.layers.some((l) => l.id === message.id)) return draft;
      return { ...draft, layers: [{ id: message.id, visible: true }, ...draft.layers] };
    case "removeLayer":
      return { ...draft, layers: draft.layers.filter((l) => l.id !== message.id) };
    case "toggleLayer":
      return { ...draft, layers: draft.layers.map((l) => (l.id === message.id ? { ...l, visible: !l.visible } : l)) };
    case "orderLayers": {
      const ordered = message.ids.flatMap((id) => draft.layers.filter((l) => l.id === id));
      // The same layers in another order only.
      return ordered.length === draft.layers.length ? { ...draft, layers: ordered } : draft;
    }
    case "basemap":
      return { ...draft, basemap: message.basemap, basemapFailed: false };
    case "basemapFailed":
      return draft.basemap === "schematic" ? draft : { ...draft, basemap: "schematic", basemapFailed: true };
    case "opacity":
      return { ...draft, opacity: Math.min(1, Math.max(0, message.value)) };
    case "click":
      return { ...draft, clicked: { lon: message.lon, lat: message.lat } };
    case "facilities":
      // Moves belong to a layer.
      return message.id === draft.facilities ? draft : { ...draft, facilities: message.id, labelField: "", history: [], undone: [], selected: null };
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
    case "save":
      return { ...draft, saving: true };
    case "saved":
      // Later corrections build on the saved copy.
      return { ...draft, saving: false, facilities: message.id, history: [], undone: [], tags: "", selected: null };
    case "saveFailed":
      return { ...draft, saving: false };
    case "tags":
      return { ...draft, tags: message.text };
  }
}

/** The draft as shown: the first reference raster and facilities, a name-like label. */
export function withMapDefaults(
  draft: MapDraft,
  first: { reference: number | null; facilities: number | null },
  fieldsOf: (layer: number | null) => readonly string[],
): MapDraft {
  const reference = draft.reference ?? first.reference;
  const facilities = draft.facilities ?? first.facilities;
  const labelField = pickField(draft.labelField, fieldsOf(facilities), ["name", "label", "cat"]);
  if (reference === draft.reference && facilities === draft.facilities && labelField === draft.labelField) return draft;
  return { ...draft, reference, facilities, labelField };
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

/** A GeoJSON feature of `GET …/features`; `sample`: the reference raster at its point. */
export interface PointFeature {
  type: "Feature";
  id?: number | string;
  geometry: { type: string; coordinates: unknown } | null;
  properties: Record<string, unknown> | null;
  sample?: number | null;
}

export interface PointCollection {
  type: "FeatureCollection";
  features: PointFeature[];
}

/**
 * Facilities to inspect: no value of the reference raster at their point.
 * A moved facility counts by the value at its new point (`movedValues`;
 * `undefined` while unknown: to inspect until known).
 */
export function toInspect(features: readonly PointFeature[], movedValues: ReadonlyMap<number, number | null | undefined>): Set<number> {
  const out = new Set<number>();
  for (const f of features) {
    if (typeof f.id !== "number") continue;
    const value = movedValues.has(f.id) ? movedValues.get(f.id) : f.sample;
    if (value === null || value === undefined) out.add(f.id);
  }
  return out;
}

/** The facility to inspect after `selected`, in layer order, wrapping; `null` when none is left. */
export function nextToInspect(features: readonly PointFeature[], inspect: ReadonlySet<number>, selected: number | null): number | null {
  const fids = features.flatMap((f) => (typeof f.id === "number" ? [f.id] : []));
  const candidates = fids.filter((f) => inspect.has(f));
  const at = selected === null ? -1 : fids.indexOf(selected);
  return candidates.find((f) => fids.indexOf(f) > at) ?? candidates[0] ?? null;
}

/** Moved facilities in place; properties untouched, so a drag preview keeps the style. */
export function placed(collection: PointCollection, moved: readonly Move[]): PointCollection {
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

/** The collection drawn: moved facilities in place, with `inspect` and `selected` properties for their style. */
export function withMoves(collection: PointCollection, moved: readonly Move[], inspect: ReadonlySet<number>, selected: number | null): PointCollection {
  const at = placed(collection, moved);
  return {
    ...at,
    features: at.features.map((f) => {
      const fid = typeof f.id === "number" ? f.id : undefined;
      return { ...f, properties: { ...f.properties, inspect: fid !== undefined && inspect.has(fid), selected: fid !== undefined && fid === selected } };
    }),
  };
}

/** Point of a facility (a point or a one-point multipoint). */
export function pointOf(feature: PointFeature | undefined): [number, number] | null {
  const c = feature?.geometry?.coordinates;
  const flat = Array.isArray(c) ? (c.flat() as unknown[]) : [];
  return typeof flat[0] === "number" && typeof flat[1] === "number" ? [flat[0], flat[1]] : null;
}

/** MapTiler style URL of a basemap; `null`: the offline schematic map (no key, or chosen). */
export function basemapStyle(basemap: Basemap, key: string | null | undefined): string | null {
  if (basemap === "schematic" || !key) return null;
  return `https://api.maptiler.com/maps/${basemap}/style.json?key=${encodeURIComponent(key)}`;
}
