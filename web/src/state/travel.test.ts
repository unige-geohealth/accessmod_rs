import { describe, expect, it } from "vitest";
import { initialRows } from "./scenario";
import { limitSeconds, TRAVEL, travelJob, updateTravel, withTravelDefaults } from "./travel";

describe("travel draft", () => {
  it("reads the maximum travel time in minutes; 0 is no limit", () => {
    expect(limitSeconds("120")).toBe(7200);
    expect(limitSeconds("0")).toBeNull();
    expect(limitSeconds("1.5")).toBe(90);
    for (const bad of ["", "abc", "-1", "1e20"]) expect(limitSeconds(bad)).toBeUndefined();
  });

  it("keeps a draft of a wider type", () => {
    const draft = { ...TRAVEL, extra: 1 };
    const next = updateTravel(draft, { type: "knight", on: true });
    expect(next).toEqual({ ...draft, knight: true });
    expect(updateTravel(next, { type: "landCover", id: null })).toBe(next);
  });

  it("shows the first land cover until the user chooses", () => {
    expect(withTravelDefaults(TRAVEL, 2).landCover).toBe(2);
    const chosen = { ...TRAVEL, landCover: 4 };
    expect(withTravelDefaults(chosen, 2)).toBe(chosen);
  });

  it("makes the travel fields once the land cover and a speed are set", () => {
    const datasets = new Map([1, 2].map((id) => [id, { id, file: `datasets/${id}.tif` } as never]));
    const scenario = initialRows([1], new Map());
    expect(travelJob(TRAVEL, { datasets, dem: 1, scenario })).toMatchObject({
      fields: undefined,
      landCoverIssues: ["srv_analysis_accessibility_missing_merged_lc_warning"],
    });
    const slow = travelJob({ ...TRAVEL, landCover: 2 }, { datasets, dem: 1, scenario });
    expect(slow.scenarioIssues).toEqual(["analysis.scenario_no_speed"]);
    const ok = travelJob({ ...TRAVEL, landCover: 2, knight: true }, { datasets, dem: 1, scenario: [{ ...scenario[0]!, speed: 4 }] });
    expect(ok.fields).toMatchObject({ dem: "datasets/1.tif", land_cover: "datasets/2.tif", neighbourhood: 16 });
  });
});
