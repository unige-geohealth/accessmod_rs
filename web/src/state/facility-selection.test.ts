import { describe, expect, it } from "vitest";
import { ALL, checkedOnPage, checkPage, count, isChecked, toFacilitySource } from "./facility-selection";

describe("facility selection", () => {
  it("checks every matching facility by default", () => {
    expect(isChecked(ALL, 3)).toBe(true);
    expect(checkedOnPage(ALL, [1, 2])).toEqual(["1", "2"]);
    expect(count(ALL, 26000)).toBe(26000);
    expect(toFacilitySource("datasets/3.fgb", "cat", ALL)).toEqual({ path: "datasets/3.fgb", id_field: "cat" });
  });

  it("keeps checks of other pages when a page changes", () => {
    const first = checkPage(ALL, [1, 2, 3], [1, 3], [1, 2, 3, 4, 5]);
    expect(first).toEqual({ mode: "ids", where: "", ids: [4, 5, 1, 3] });
    const second = checkPage(first, [4, 5], [5]);
    expect(second.mode === "ids" && second.ids).toEqual([1, 3, 5]);
    expect(checkedOnPage(second, [1, 2, 3])).toEqual(["1", "3"]);
    expect(isChecked(second, "3")).toBe(true);
    expect(count(second, 5)).toBe(3);
  });

  it("writes the filter and the checked ids to the job", () => {
    const selection = checkPage({ mode: "all", where: "type = 'H'" }, ["A-1", "A-2"], ["A-2"], ["A-1", "A-2"]);
    expect(toFacilitySource("hf.fgb", "code", selection)).toEqual({ path: "hf.fgb", id_field: "code", where: "type = 'H'", ids: ["A-2"] });
  });
});
