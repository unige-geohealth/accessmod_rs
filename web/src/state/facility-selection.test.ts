import { kindOf, match, opsFor, shownQuery, withField } from "@accessmod/ui/am/selection-toolbar";
import { describe, expect, it } from "vitest";
import {
  ALL,
  checkedOnPage,
  checkPage,
  count,
  facilityIssues,
  facilityRow,
  isChecked,
  NO_FACILITIES,
  selectMatching,
  toFacilitySource,
  unselectMatching,
  updateFacilities,
  withFacilityDefaults,
} from "./facility-selection";

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

describe("facility input", () => {
  it("shows the first layer and the cat field until the user chooses", () => {
    const fieldsOf = (id: number | null) => (id === 3 ? ["name", "cat"] : ["code"]);
    expect(withFacilityDefaults(NO_FACILITIES, 3, fieldsOf)).toMatchObject({ layer: 3, idField: "cat" });
    expect(withFacilityDefaults({ ...NO_FACILITIES, layer: 5 }, 3, fieldsOf)).toMatchObject({ layer: 5, idField: "code" });
    const chosen = { ...NO_FACILITIES, layer: 3, idField: "name" };
    expect(withFacilityDefaults(chosen, 3, fieldsOf)).toBe(chosen);
  });

  it("resets the field and the selection with the layer", () => {
    const input = { layer: 3, idField: "cat", selection: { mode: "ids" as const, where: "a", ids: [1] } };
    expect(updateFacilities(input, { type: "layer", id: 3 })).toBe(input);
    expect(updateFacilities(input, { type: "idField", field: "code" }).selection).toEqual({ mode: "all", where: "a" });
    expect(facilityIssues(NO_FACILITIES, undefined)).toEqual(["srv_analysis_accessibility_missing_facility_layer"]);
  });
});

describe("select and unselect where (AM5 toolbar)", () => {
  const every = [1, 2, 3, 4];

  it("unchecks the matching facilities, from all or from checked ids", () => {
    expect(unselectMatching(ALL, [2, 4], every)).toEqual({ mode: "ids", where: "", ids: [1, 3] });
    expect(unselectMatching({ mode: "ids", where: "", ids: [1, 2] }, ["2"], every)).toEqual({ mode: "ids", where: "", ids: [1] });
    expect(unselectMatching(ALL, [], every)).toEqual(ALL);
  });

  it("checks the matching facilities too, and every one is all again", () => {
    const some = { mode: "ids" as const, where: "w", ids: [1] };
    expect(selectMatching(some, [3, 1], every)).toEqual({ mode: "ids", where: "w", ids: [1, 3] });
    expect(selectMatching(some, [2, 3, 4], every)).toEqual({ mode: "all", where: "w" });
    expect(selectMatching(ALL, [2], every)).toBe(ALL);
  });
});

describe("facility row", () => {
  const speeds = new Map([
    [3, 0],
    [1003, 5],
  ]);
  it("computes the AccessMod columns from the samples and the scenario", () => {
    const row = facilityRow(["A", { a: 1 }], ["osm_id", "tags"], [3, 12.5], { landCover: true, popAt: 1, speeds });
    expect(row).toEqual({ amOnBarrier: "no", amOnZero: "yes", amCatLandCover: 3, amPopCell: 12.5, osm_id: "A", tags: '{"a":1}' });
    expect(facilityRow([1], ["cat"], [null], { landCover: true, speeds })).toEqual({ amOnBarrier: "yes", amOnZero: "no", amCatLandCover: null, cat: 1 });
  });

  it("matches computed columns as the table shows them (the reported bug)", () => {
    const rows = [
      facilityRow(["a"], ["osm_id"], [1003], { landCover: true, speeds }),
      facilityRow(["b"], ["osm_id"], [null], { landCover: true, speeds }),
    ];
    expect(rows.filter((r) => match(r.amOnBarrier, "=", "no", "text")).map((r) => r.osm_id)).toEqual(["a"]);
    expect(rows.filter((r) => match(r.amCatLandCover, ">=", "1000", kindOf(rows, "amCatLandCover"))).map((r) => r.osm_id)).toEqual(["a"]);
  });
});

describe("selection toolbar condition", () => {
  it("offers like for text and comparisons for numbers", () => {
    expect(opsFor("text")).toEqual(["=", "!=", "like"]);
    expect(opsFor("number")).toEqual(["=", "!=", ">", "<", ">=", "<="]);
    expect(kindOf([{ a: null }, { a: 2 }], "a")).toBe("number");
    expect(kindOf([{ a: "2" }], "a")).toBe("text");
  });

  it("compares as AM5 (Tabulator searchRows)", () => {
    expect(match("Neno Rural Hospital", "like", "HOSP", "text")).toBe(true);
    expect(match("Clinic", "like", "hosp", "text")).toBe(false);
    expect(match(1003, "=", "1003", "number")).toBe(true);
    expect(match(9, ">", "10", "number")).toBe(false);
    expect(match("9", ">", "10", "text")).toBe(true);
    expect(match(3, "<=", "3", "number")).toBe(true);
    expect(match(null, "=", "x", "text")).toBe(false);
    expect(match(null, "!=", "x", "text")).toBe(true);
    expect(match(null, "<", "3", "number")).toBe(false);
  });

  it("shows the first column, an operator of its kind and the first value", () => {
    const columns = [
      { field: "name", kind: "text" as const },
      { field: "amOnBarrier", kind: "text" as const, values: ["no", "yes"] },
      { field: "pop", kind: "number" as const },
    ];
    expect(shownQuery({ field: "", op: "=", value: "" }, columns).query).toEqual({ field: "name", op: "=", value: "" });
    expect(shownQuery({ field: "amOnBarrier", op: "=", value: "" }, columns).query.value).toBe("no");
    expect(shownQuery({ field: "amOnBarrier", op: "like", value: "y" }, columns).query.value).toBe("y");
    expect(shownQuery({ field: "pop", op: "like", value: "3" }, columns).query.op).toBe("=");
    expect(withField({ field: "name", op: "like", value: "x" }, "pop")).toEqual({ field: "pop", op: "like", value: "" });
  });
});
