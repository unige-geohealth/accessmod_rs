import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { EMPTY, incompleteRuns, selectedDatasets, selectedKeys, selectedRuns, tagChanges, updateData } from "./data-document";

const datasets = [
  { id: 1, tags: ["demo"] },
  { id: 2, tags: ["health"] },
] as Api.Dataset[];

describe("data document", () => {
  it("keeps the state when nothing changes", () => {
    const doc = updateData(EMPTY, { type: "select", run: null, keys: ["d2"] });
    expect(updateData(doc, { type: "select", run: null, keys: ["d2"] })).toBe(doc);
    expect(updateData(doc, { type: "select", run: 1, keys: ["d2"] })).not.toBe(doc);
    expect(updateData(EMPTY, { type: "discardEdits" })).toBe(EMPTY);
  });

  it("commits only tag edits that change the tags", () => {
    let doc = updateData(EMPTY, { type: "editTags", id: 1, text: " demo " });
    doc = updateData(doc, { type: "editTags", id: 2, text: "health, Santé" });
    expect(tagChanges(doc, datasets)).toEqual([{ id: 2, text: "health, Santé", tags: ["health", "Sante"] }]);
  });

  it("keeps an edit typed while the previous one was committed", () => {
    let doc = updateData(EMPTY, { type: "editTags", id: 2, text: "alpha" });
    const changes = tagChanges(doc, datasets);
    doc = updateData(doc, { type: "editTags", id: 2, text: "beta" });
    doc = updateData(doc, { type: "renamed", changes });
    expect(doc.edits).toEqual({ 2: "beta" });
    expect(updateData(doc, { type: "renamed", changes: tagChanges(doc, datasets) }).edits).toEqual({});
  });

  it("forgets settled datasets", () => {
    let doc = updateData(EMPTY, { type: "select", run: null, keys: ["d1", "j2", "d2"] });
    doc = updateData(doc, { type: "editTags", id: 2, text: "x" });
    doc = updateData(doc, { type: "removed", ids: [2] });
    expect(doc).toEqual({ selected: { run: null, keys: ["d1", "j2"] }, edits: {} });
  });

  it("selects only datasets that still exist", () => {
    const doc = updateData(EMPTY, { type: "select", run: null, keys: ["d2", "d9", "j1"] });
    expect(selectedDatasets(selectedKeys(doc, ["d2", "d9", "j1"], null), datasets).map((d) => d.id)).toEqual([2]);
  });

  it("selects the rows of the run filter until the user selects under it", () => {
    expect(selectedKeys(EMPTY, ["j3", "d1"], 3)).toEqual(["j3", "d1"]);
    expect(selectedKeys(EMPTY, ["d1"], null)).toEqual([]);
    const chosen = updateData(EMPTY, { type: "select", run: 3, keys: ["d1"] });
    expect(selectedKeys(chosen, ["j3", "d1"], 3)).toEqual(["d1"]);
    // Another run (a new filter, Back): its rows again, never the other run's choice.
    expect(selectedKeys(chosen, ["j4", "d2"], 4)).toEqual(["j4", "d2"]);
    expect(selectedKeys(chosen, ["d1", "d2"], null)).toEqual([]);
  });

  it("tells the selected jobs an archive cannot replay", () => {
    const runs = [{ id: 3, inputs: [1, 2] }, { id: 4, inputs: [1] }] as Api.RunRecord[];
    expect(incompleteRuns(["j3", "j4", "d1"], runs).map((r) => r.id)).toEqual([3]);
    expect(incompleteRuns(["d2", "d1"], runs)).toEqual([]);
    expect(selectedRuns(["d1", "j3", "j4"])).toEqual([3, 4]);
  });

  it("never selects a hidden row", () => {
    const chosen = updateData(EMPTY, { type: "select", run: null, keys: ["d1", "d2"] });
    expect(selectedKeys(chosen, ["d2", "d3"], null)).toEqual(["d2"]);
  });
});
