import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { EMPTY, selectedDatasets, selectedKeys, tagChanges, updateData } from "./data-document";

const datasets = [
  { id: 1, tags: ["demo"] },
  { id: 2, tags: ["health"] },
] as Api.Dataset[];

describe("data document", () => {
  it("keeps the state when nothing changes", () => {
    const doc = updateData(EMPTY, { type: "select", keys: ["d2"] });
    expect(updateData(doc, { type: "select", keys: ["d2"] })).toBe(doc);
    expect(updateData(EMPTY, { type: "filtered" })).toBe(EMPTY);
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
    let doc = updateData(EMPTY, { type: "select", keys: ["d1", "j2", "d2"] });
    doc = updateData(doc, { type: "editTags", id: 2, text: "x" });
    doc = updateData(doc, { type: "removed", ids: [2] });
    expect(doc).toEqual({ selected: ["d1", "j2"], edits: {} });
  });

  it("selects only datasets that still exist", () => {
    const doc = updateData(EMPTY, { type: "select", keys: ["d2", "d9", "j1"] });
    expect(selectedDatasets(selectedKeys(doc, [], false), datasets).map((d) => d.id)).toEqual([2]);
  });

  it("selects the rows of the run filter until the user selects", () => {
    expect(selectedKeys(EMPTY, ["j3", "d1"], true)).toEqual(["j3", "d1"]);
    expect(selectedKeys(EMPTY, ["d1"], false)).toEqual([]);
    const chosen = updateData(EMPTY, { type: "select", keys: ["d1"] });
    expect(selectedKeys(chosen, ["j3", "d1"], true)).toEqual(["d1"]);
    expect(selectedKeys(updateData(chosen, { type: "filtered" }), ["j3", "d1"], true)).toEqual(["j3", "d1"]);
  });
});
