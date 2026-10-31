import { describe, expect, it } from "vitest";
import { EMPTY, type MapDraft, moves, saveIssues, updateMap, withMapDefaults, withMoves } from "./map-tool";

const apply = (draft: MapDraft, ...messages: Parameters<typeof updateMap>[1][]) => messages.reduce(updateMap, draft);

describe("interactive map", () => {
  it("keeps the latest move of each facility, with undo and redo", () => {
    let draft = apply(
      { ...EMPTY, facilities: 3 },
      { type: "move", move: { fid: 1, lon: 1, lat: 1 } },
      { type: "move", move: { fid: 2, lon: 2, lat: 2 } },
      { type: "move", move: { fid: 1, lon: 5, lat: 5 } },
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
    draft = apply(draft, { type: "move", move: { fid: 9, lon: 0, lat: 0 } }, { type: "redo" });
    expect(draft.undone).toEqual([]);
    // Another layer: nothing moved.
    expect(moves(apply(draft, { type: "facilities", id: 4 }))).toEqual([]);
  });

  it("checks Save as AM5 and resets after it", () => {
    expect(saveIssues(EMPTY)).toEqual(["tool_map_relocate_missing_layer", "tool_map_relocate_missing_tags"]);
    const draft = apply({ ...EMPTY, facilities: 3, editing: true }, { type: "tags", text: "moved" }, { type: "move", move: { fid: 1, lon: 1, lat: 1 } });
    expect(saveIssues(draft)).toEqual([]);
    expect(apply(draft, { type: "saved" })).toMatchObject({ history: [], editing: false, tags: "" });
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
