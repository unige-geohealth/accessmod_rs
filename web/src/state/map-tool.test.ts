import { describe, expect, it } from "vitest";
import {
  basemapStyle,
  EMPTY,
  MAX_LAYERS,
  type MapDraft,
  moves,
  nextToInspect,
  type PointFeature,
  pointOf,
  saveIssues,
  toInspect,
  updateMap,
  withMapDefaults,
  withMoves,
} from "./map-tool";

const apply = (draft: MapDraft, ...messages: Parameters<typeof updateMap>[1][]) => messages.reduce(updateMap, draft);

describe("interactive map", () => {
  it("keeps the latest move of each facility, with undo and redo", () => {
    let draft = apply(
      { ...EMPTY, facilities: 3 },
      { type: "move", layer: 3, move: { fid: 1, lon: 1, lat: 1 } },
      { type: "move", layer: 3, move: { fid: 2, lon: 2, lat: 2 } },
      { type: "move", layer: 3, move: { fid: 1, lon: 5, lat: 5 } },
    );
    expect(moves(draft)).toEqual([
      { fid: 1, lon: 5, lat: 5 },
      { fid: 2, lon: 2, lat: 2 },
    ]);
    draft = apply(draft, { type: "undo" }, { type: "undo" });
    expect(moves(draft)).toEqual([{ fid: 1, lon: 1, lat: 1 }]);
    draft = apply(draft, { type: "redo" });
    expect(moves(draft)).toHaveLength(2);
    // A new move drops what was undone.
    draft = apply(draft, { type: "move", layer: 3, move: { fid: 9, lon: 0, lat: 0 } }, { type: "redo" });
    expect(draft.undone).toEqual([]);
    // Another layer: nothing moved.
    expect(moves(apply(draft, { type: "facilities", id: 4 }))).toEqual([]);
  });

  it("pins the layer of the moves: a new default layer does not inherit them", () => {
    // The default layer (stored null) is pinned by the first move.
    const draft = apply(EMPTY, { type: "move", layer: 3, move: { fid: 1, lon: 1, lat: 1 } });
    expect(draft.facilities).toBe(3);
    expect(withMapDefaults(draft, { reference: null, facilities: 4 }, () => []).facilities).toBe(3);
    // A move on another layer starts over.
    expect(moves(apply(draft, { type: "move", layer: 4, move: { fid: 2, lon: 2, lat: 2 } }))).toEqual([{ fid: 2, lon: 2, lat: 2 }]);
  });

  it("checks Save as AM5; a save holds the edits, then edits its copy", () => {
    expect(saveIssues(EMPTY)).toEqual(["tool_map_relocate_missing_layer", "tool_map_relocate_missing_tags"]);
    const draft = apply({ ...EMPTY, facilities: 3 }, { type: "tags", text: "moved" }, { type: "move", layer: 3, move: { fid: 1, lon: 1, lat: 1 } });
    expect(saveIssues(draft)).toEqual([]);
    // While saving: no move, undo, redo or other layer.
    const saving = apply(draft, { type: "save" });
    for (const m of [{ type: "move", layer: 3, move: { fid: 2, lon: 2, lat: 2 } }, { type: "undo" }, { type: "facilities", id: 4 }] as const) {
      expect(updateMap(saving, m)).toBe(saving);
    }
    // Saved: the copy is shown, nothing pending.
    expect(apply(saving, { type: "saved", id: 9 })).toMatchObject({ facilities: 9, history: [], saving: false, tags: "" });
    // Failed: the moves stay to try again.
    expect(moves(apply(saving, { type: "saveFailed" }))).toEqual([{ fid: 1, lon: 1, lat: 1 }]);
  });

  it("takes defaults and places moved points", () => {
    const draft = withMapDefaults(EMPTY, { reference: 2, facilities: 3 }, () => ["cat", "name"]);
    expect([draft.reference, draft.facilities, draft.labelField]).toEqual([2, 3, "name"]);
    const collection = {
      type: "FeatureCollection" as const,
      features: [
        { type: "Feature" as const, id: 0, geometry: { type: "Point", coordinates: [0, 0] }, properties: {} },
        { type: "Feature" as const, id: 1, geometry: { type: "Point", coordinates: [1, 1] }, properties: {} },
      ],
    };
    expect(withMoves(collection, [{ fid: 1, lon: 7, lat: 8 }]).features[1]?.geometry?.coordinates).toEqual([7, 8]);
  });

  const point = (id: number, sample: number | null, coordinates: unknown = [id, id]): PointFeature => ({
    type: "Feature",
    id,
    geometry: { type: "Point", coordinates },
    properties: {},
    sample,
  });

  it("lists the facilities to inspect, moved ones by their new value", () => {
    const features = [point(0, 1), point(1, null), point(2, null), point(3, 4)];
    expect([...toInspect(features, new Map())]).toEqual([1, 2]);
    // 1 moved on a value, 3 moved off the grid, 0 moved and not yet known.
    const moved = new Map<number, number | null | undefined>([
      [1, 5],
      [3, null],
      [0, undefined],
    ]);
    expect([...toInspect(features, moved)].sort()).toEqual([0, 2, 3]);
  });

  it("goes to the next facility to inspect, in layer order, wrapping", () => {
    const features = [point(0, 1), point(1, null), point(2, 1), point(3, null)];
    const inspect = toInspect(features, new Map());
    expect(nextToInspect(features, inspect, null)).toBe(1);
    expect(nextToInspect(features, inspect, 1)).toBe(3);
    expect(nextToInspect(features, inspect, 3)).toBe(1);
    expect(nextToInspect(features, new Set(), 1)).toBeNull();
  });

  it("styles facilities to inspect and the selected one", () => {
    const drawn = withMoves({ type: "FeatureCollection", features: [point(0, null), point(1, 1)] }, [{ fid: 1, lon: 9, lat: 9 }], new Set([0]), 1);
    expect(drawn.features.map((f) => f.properties)).toEqual([
      { inspect: true, selected: false },
      { inspect: false, selected: true },
    ]);
    expect(drawn.features[1]?.geometry?.coordinates).toEqual([9, 9]);
    expect(pointOf(point(5, 1, [[3, 4]]))).toEqual([3, 4]);
    expect(pointOf(undefined)).toBeNull();
  });

  it("keeps at most ten layers and needs a key for MapTiler", () => {
    const shown = Array.from({ length: 12 }, (_, i) => i);
    expect(updateMap(EMPTY, { type: "layers", shown }).layers).toHaveLength(MAX_LAYERS);
    expect(basemapStyle("satellite-v4", null)).toBeNull();
    expect(basemapStyle("schematic", "k")).toBeNull();
    expect(basemapStyle("satellite-v4", "a b")).toBe("https://api.maptiler.com/maps/satellite-v4/style.json?key=a%20b");
    // Offline: back to the schematic map until another basemap is chosen.
    const failed = updateMap({ ...EMPTY, basemap: "satellite-v4" }, { type: "basemapFailed" });
    expect([failed.basemap, failed.basemapFailed]).toEqual(["schematic", true]);
    expect(updateMap(failed, { type: "basemap", basemap: "outdoor-v4" }).basemapFailed).toBe(false);
  });
});
