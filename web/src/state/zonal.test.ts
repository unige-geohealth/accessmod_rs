import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { buildJob, EMPTY, limitsOf, updateZonal, zonalOutput, zonalTable } from "./zonal";

const datasets = new Map(
  [
    { id: 1, class: "rDem", file: "datasets/1.tif" },
    { id: 2, class: "rPopulation", file: "datasets/2.tif" },
    { id: 3, class: "rTravelTime", file: "runs/1/travel_time.tif" },
    { id: 4, class: "vZone", file: "datasets/4.fgb" },
    { id: 5, class: "tZonalStat", file: "runs/2/zonal_statistics.csv" },
  ].map((d) => [d.id, d as Api.Dataset]),
);

describe("zonal statistics draft", () => {
  it("reads minutes as AM5: comma separated, invalid and duplicated values ignored", () => {
    expect(limitsOf("120, 10,abc, 20, 10, -5, 0")).toEqual([600, 1200, 7200]);
    expect(limitsOf("0")).toEqual([]);
  });

  it("lists what is missing, then builds the job", () => {
    expect(buildJob(EMPTY, { datasets, dem: 1 }).issues).toEqual([
      "srv_analysis_accessibility_select_population",
      "srv_analysis_accessibility_missing_travel_time",
      "srv_analysis_accessibility_missing_zone",
      "analysis.time_limits_missing",
    ]);
    let draft = EMPTY;
    for (const message of [
      { type: "population", id: 2 },
      { type: "travelTime", id: 3 },
      { type: "zones", message: { type: "layer", id: 4 } },
      { type: "zones", message: { type: "idField", field: "cat" } },
      { type: "zones", message: { type: "labelField", field: "name" } },
      { type: "minutes", text: "60, 30" },
    ] as const) {
      draft = updateZonal(draft, message);
    }
    expect(buildJob(draft, { datasets, dem: 1 }).document).toEqual({
      schema_version: 1,
      dem: "datasets/1.tif",
      population: "datasets/2.tif",
      travel_time: "runs/1/travel_time.tif",
      zones: { path: "datasets/4.fgb", id_field: "cat", label_field: "name" },
      time_limits_s: [1800, 3600],
    });
  });

  it("finds the output table and pivots it to one column per time step", () => {
    expect(zonalOutput({ datasets: [3, 5] } as Api.RunRecord, datasets)).toBe(5);
    const rows = {
      columns: ["zone_id", "label", "time_limit_s", "population", "population_covered", "coverage_percent"],
      rows: [
        [1, "north", 1800, 100, 10, 10],
        [2, "south", 1800, 50, 25, 50],
        [1, "north", 3600, 100, 40, 40],
        [2, "south", 3600, 50, 50, 100],
      ],
      offset: 0,
      total: 4,
    } as Api.Rows;
    expect(zonalTable(rows, false).data).toHaveLength(4);
    expect(zonalTable(rows, true)).toEqual({
      columns: ["zone_id", "label", "population", "population_covered_30min", "coverage_percent_30min", "population_covered_60min", "coverage_percent_60min"],
      data: [
        { zone_id: 1, label: "north", population: 100, population_covered_30min: 10, coverage_percent_30min: 10, population_covered_60min: 40, coverage_percent_60min: 40 },
        { zone_id: 2, label: "south", population: 50, population_covered_30min: 25, coverage_percent_30min: 50, population_covered_60min: 50, coverage_percent_60min: 100 },
      ],
    });
  });
});
