import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { EMPTY, selectedDatasets, tagChanges, updateData } from "./data-document";

const datasets = [
  { id: 1, tags: ["demo"] },
  { id: 2, tags: ["health"] },
] as Api.Dataset[];

describe("data document", () => {
  it("keeps the state when nothing changes", () => {
    const doc = updateData(EMPTY, { type: "select", ids: [2] });
    expect(updateData(doc, { type: "select", ids: [2] })).toBe(doc);
    expect(updateData(EMPTY, { type: "discardEdits" })).toBe(EMPTY);
  });

  it("commits only tag edits that change the tags", () => {
    let doc = updateData(EMPTY, { type: "editTags", id: 1, text: " demo " });
    doc = updateData(doc, { type: "editTags", id: 2, text: "health, Santé" });
    expect(tagChanges(doc, datasets)).toEqual([{ id: 2, tags: ["health", "Sante"] }]);
  });

  it("forgets settled datasets", () => {
    let doc = updateData(EMPTY, { type: "select", ids: [1, 2] });
    doc = updateData(doc, { type: "editTags", id: 2, text: "x" });
    doc = updateData(doc, { type: "settled", ids: [2] });
    expect(doc).toEqual({ selected: [1], edits: {} });
  });

  it("selects only datasets that still exist", () => {
    const doc = updateData(EMPTY, { type: "select", ids: [2, 9] });
    expect(selectedDatasets(doc, datasets).map((d) => d.id)).toEqual([2]);
  });
});
