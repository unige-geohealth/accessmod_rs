import { describe, expect, it } from "vitest";
import { EMPTY, type MapDraft, moves, saveIssues, updateMap, withMapDefaults, withMoves } from "./map-tool";

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
    expect(withMapDefaults(draft, { raster: null, facilities: 4 }, () => []).facilities).toBe(3);
    // A move on another layer starts over.
    expect(moves(apply(draft, { type: "move", layer: 4, move: { fid: 2, lon: 2, lat: 2 } }))).toEqual([{ fid: 2, lon: 2, lat: 2 }]);
  });

  it("checks Save as AM5; a save clears the moves it sent only", () => {
    expect(saveIssues(EMPTY)).toEqual(["tool_map_relocate_missing_layer", "tool_map_relocate_missing_tags"]);
    const draft = apply({ ...EMPTY, facilities: 3 }, { type: "tags", text: "moved" }, { type: "move", layer: 3, move: { fid: 1, lon: 1, lat: 1 } });
    expect(saveIssues(draft)).toEqual([]);
    expect(apply(draft, { type: "saved", layer: 3, count: 1 })).toMatchObject({ history: [], tags: "" });
    // A move made while saving stays.
    const later = apply(draft, { type: "move", layer: 3, move: { fid: 2, lon: 2, lat: 2 } }, { type: "saved", layer: 3, count: 1 });
    expect([moves(later), later.tags]).toEqual([[{ fid: 2, lon: 2, lat: 2 }], "moved"]);
    // Another layer since: unchanged.
    const other = apply(draft, { type: "facilities", id: 4 });
    expect(apply(other, { type: "saved", layer: 3, count: 1 })).toBe(other);
  });

  it("takes defaults and places moved points", () => {
    const draft = withMapDefaults(EMPTY, { raster: 2, facilities: 3 }, () => ["cat", "name"]);
    expect([draft.raster, draft.facilities, draft.labelField]).toEqual([2, 3, "name"]);
    const collection = {
      type: "FeatureCollection" as const,
      features: [
        { type: "Feature" as const, id: 0, geometry: { type: "Point", coordinates: [0, 0] }, properties: {} },
        { type: "Feature" as const, id: 1, geometry: { type: "Point", coordinates: [1, 1] }, properties: {} },
      ],
    };
    expect(withMoves(collection, [{ fid: 1, lon: 7, lat: 8 }]).features[1]?.geometry?.coordinates).toEqual([7, 8]);
  });
});
