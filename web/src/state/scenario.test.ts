import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { editRow, initialRows, labelsOf, landCoverClasses, mergeTable, tableNotices, tableRows, toScenario } from "./scenario";

const rows = (columns: string[], data: unknown[][]): Api.Rows => ({ columns, rows: data, offset: 0, total: data.length });

const landCover = {
  id: 2,
  class: "rLandCoverMerged",
  tags: ["base"],
  summary: { kind: "raster", stats: { values: [{ value: 3, count: 1 }, { value: 1, count: 9 }] } },
} as unknown as Api.Dataset;

describe("scenario", () => {
  it("starts from the land cover classes, as AM5", () => {
    expect(landCoverClasses(landCover)).toEqual([1, 3]);
    const labels = labelsOf(rows(["Class", "Label"], [["1", "forest"], ["x", "bad"]]));
    expect(initialRows([1, 3], labels)).toEqual([
      { class: 1, label: "forest", speed: 0, mode: "WALKING" },
      { class: 3, label: "no_label_3", speed: 0, mode: "WALKING" },
    ]);
  });

  it("merges a scenario table: matched rows first-wins, missing classes kept, by class", () => {
    const current = initialRows([1, 2, 3], new Map());
    const table = tableRows(
      rows(
        ["class", "label", "speed", "mode"],
        [
          ["3", "road", "60", "motorized"],
          ["9", "lake", "5", "WALKING"],
          ["1", "forest", "4", "boat"],
          ["1", "again", "8", "WALKING"],
        ],
      ),
    );
    expect(mergeTable(current, table)).toEqual([
      { class: 1, label: "forest", speed: 4, mode: "WALKING" },
      { class: 2, label: "no_label_2", speed: 0, mode: "WALKING" },
      { class: 3, label: "road", speed: 60, mode: "MOTORIZED" },
    ]);
  });

  it("reports unknown modes and unmatched classes or labels", () => {
    const current = initialRows([1], new Map([[1, "forest"]]));
    expect(tableNotices(current, rows(["class", "label", "speed", "mode"], [["1", "forest", "4", "WALKING"]]))).toEqual({
      unknownMode: false,
      unmatched: false,
    });
    expect(tableNotices(current, rows(["class", "label", "speed", "mode"], [["2", "x", "4", "boat"]]))).toEqual({
      unknownMode: true,
      unmatched: true,
    });
  });

  it("edits speed and mode, ignoring invalid values", () => {
    const current = initialRows([1], new Map());
    expect(editRow(current, 1, "speed", "12.5")[0]?.speed).toBe(12.5);
    expect(editRow(current, 1, "speed", "-3")[0]?.speed).toBe(0);
    expect(editRow(current, 1, "speed", "abc")).toEqual(current);
    expect(editRow(current, 1, "mode", "motorized")[0]?.mode).toBe("MOTORIZED");
    expect(editRow(current, 1, "mode", "boat")).toEqual(current);
  });

  it("leaves classes without speed out of the job", () => {
    const current = [
      { class: 1, label: "forest", speed: 4, mode: "WALKING" as const },
      { class: 2, label: "lake", speed: 0, mode: "WALKING" as const },
    ];
    const scenario = toScenario(current);
    expect(scenario).toEqual([{ class: 1, label: "forest", speed_kmh: 4, mode: "walking" }]);
  });
});
