import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { buildJob, EMPTY, type ScalingUpDraft, shownRows, UNLIMITED_FACILITIES, updateScalingUp } from "./scaling-up";
import { capacityChecks, capacityRows, editCapacity, exclusionRows, factorOptions, factorRows, layerOf } from "./scaling-up-tables";
import { initialRows } from "./scenario";
import type { Context } from "./travel";

const list = [
  { id: 1, class: "rDem", tags: ["demo"], file: "datasets/1.tif" },
  { id: 2, class: "rLandCoverMerged", tags: ["merged"], file: "datasets/2.tif" },
  { id: 3, class: "rPopulation", tags: ["demo"], file: "datasets/3.tif" },
  { id: 4, class: "vFacility", tags: ["demo"], file: "datasets/4.fgb" },
  { id: 5, class: "vRoad", tags: ["demo", "main"], file: "datasets/5.fgb" },
  { id: 6, class: "rPopulationResidual", tags: ["cov"], file: "datasets/6.tif" },
] as Api.Dataset[];
const datasets = new Map(list.map((d) => [d.id, d]));
const scenario = initialRows([1], new Map()).map((r) => ({ ...r, speed: 4 }));
const context: Context = { datasets, dem: 1, scenario };
const page = (columns: string[], rows: unknown[][]): Api.Rows => ({ columns, rows, offset: 0, total: rows.length }) as Api.Rows;
const capacities = [
  { min: 0, max: 1000, label: "small", capacity: 500 },
  { min: 1001, max: 5000, label: "large", capacity: 3000 },
];
const ready: ScalingUpDraft = {
  ...EMPTY,
  landCover: 2,
  population: 3,
  facilities: { layer: 4, idField: "cat", selection: { mode: "all", where: "" } },
  capacities,
  tags: "up",
};

describe("scaling-up tables", () => {
  it("reads AM5 tables, layers by class and tags", () => {
    expect(capacityRows(page(["min", "max", "label", "capacity"], [[0, 1000, "small", 500]]))).toEqual([capacities[0]]);
    expect(layerOf("vRoad__main_demo", list)).toEqual({ dataset: 5 });
    expect(layerOf("vOutputFacility", list)).toBe("facilities");
    expect(layerOf("vRoad__other", list)).toEqual({ missing: "vRoad__other" });
    const factors = factorRows(
      page(["factor", "layer", "weight", "options"], [
        ["traveltime", "vRoad__demo_main", 2, "d=from;t=aniso;k=n16;p=hvls"],
        ["unknown", "x", 1, ""],
      ]),
      list,
    );
    expect(factors).toEqual([
      { id: 1, factor: "traveltime", layer: { dataset: 5 }, weight: 2, prefer: "hvls", radiusKm: 1, method: "aniso", direction: "from", knight: true },
    ]);
    expect(factorOptions(factors[0]!)).toBe("d=from;t=aniso;k=n16;p=hvls");
    expect(exclusionRows(page(["layer", "buffer", "method"], [["vOutputFacility", 5, "keep_outside"]]), list)).toEqual([
      { id: 1, layer: "facilities", bufferKm: 5, method: "keep_outside" },
    ]);
  });

  it("edits a capacity cell", () => {
    expect(editCapacity(capacities, 1, "max", "6000")[1]).toEqual({ ...capacities[1], max: 6000 });
    expect(editCapacity(capacities, 0, "label", "tiny")[0]!.label).toBe("tiny");
    expect(editCapacity(capacities, 0, "min", "")[0]!.min).toBeNaN();
  });

  it("checks the capacity table as AM5", () => {
    expect(capacityChecks(capacities)).toEqual({ issues: [], notices: [] });
    expect(capacityChecks([{ min: 0, max: 0, label: "label", capacity: 0 }]).issues).toEqual(["srv_analysis_accessibility_scaleup_table_min_max_equality"]);
    const overlapping = [capacities[0]!, { ...capacities[1]!, min: 900, capacity: 6000 }];
    expect(capacityChecks(overlapping)).toEqual({
      issues: ["srv_analysis_accessibility_scaleup_min_value_greater_previous_max"],
      notices: ["srv_analysis_accessibility_scaleup_values_not_in_range"],
    });
  });
});

describe("scaling-up draft", () => {
  it("builds the scaling-up job from AM5's initial rows, limits 0 as AM5's unlimited", () => {
    const built = buildJob(ready, context, shownRows(ready, {}));
    expect(built.issues).toEqual([]);
    expect(built.notices).toEqual(["srv_analysis_accessibility_processing_unlimited", "srv_analysis_accessibility_new_facilities_unlimited", "srv_analysis_accessibility_selected_facilities_verification"]);
    expect(built.document).toEqual({
      schema_version: 1,
      dem: "datasets/1.tif",
      land_cover: "datasets/2.tif",
      scenario: [{ class: 1, label: "no_label_1", speed_kmh: 4, mode: "walking" }],
      method: "anisotropic",
      mobility_model: "accessmod5-v1",
      neighbourhood: 8,
      facility_issues: "error",
      population: "datasets/3.tif",
      population_residual: null,
      facilities: { path: "datasets/4.fgb", id_field: "cat" },
      direction: "towards_facilities",
      limit_s: 7200,
      band_s: 60,
      capacities,
      suitability: [{ factor: "population", layer: "population", weight: 1, prefer: "high", window_m: 1000 }],
      exclusions: [{ layer: "facilities", buffer_m: 5000, keep: "inside" }],
      max_facilities: UNLIMITED_FACILITIES,
      max_coverage_percent: 80,
      max_duration_s: null,
      catchments: true,
    });
  });

  it("starts empty, reports missing layers and the residual population", () => {
    let draft = ready;
    for (const message of [
      { type: "useExisting", on: false },
      { type: "residual", id: 6 },
      { type: "factors", rows: [{ id: 1, factor: "dist", layer: "facilities", weight: 1, prefer: "hvms", radiusKm: 1, method: "iso", direction: "to", knight: false }] },
      { type: "exclusions", rows: [{ id: 1, layer: { missing: "vExclusion__gone" }, bufferKm: 0, method: "keep_inside" }] },
      { type: "limit", limit: "newFacilities", text: "3" },
      { type: "limit", limit: "maxMinutes", text: "2" },
    ] as const) {
      draft = updateScalingUp(draft, message);
    }
    const built = buildJob(draft, context, shownRows(draft, {}));
    expect(built.issues).toEqual([
      "srv_analysis_accessibility_suitability_table_add_non_dynamic_layer",
      { key: "srv_analysis_accessibility_exclusion_table_missing_layer", args: ["vExclusion__gone"] },
    ]);
    expect(built.notices).toEqual(["srv_analysis_accessibility_residual_pop_no_facilities"]);
    const fixed = updateScalingUp(updateScalingUp(draft, { type: "reset", table: "exclusions" }), { type: "useExisting", on: true });
    expect(buildJob(fixed, context, shownRows(fixed, {})).document).toMatchObject({
      population_residual: "datasets/6.tif",
      max_facilities: 3,
      max_duration_s: 120,
      suitability: [{ factor: "distance", layer: "facilities", weight: 1, prefer: "high" }],
    });
  });

  it("waits for a selected table, refuses one longer than a read", () => {
    const fresh = { ...ready, capacities: null };
    expect(buildJob(fresh, context, shownRows(fresh, {}), { capacities: "pending", factors: "too_large" }).issues).toEqual([
      "analysis.table_loading",
      "analysis.table_too_large",
      "srv_analysis_accessibility_scaleup_table_min_max_equality",
    ]);
    // Edited rows no longer depend on their table.
    expect(buildJob(ready, context, shownRows(ready, {}), { capacities: "pending" }).issues).toEqual([]);
  });

  it("reads a newly selected table again", () => {
    const edited = updateScalingUp(ready, { type: "table", table: "capacities", id: 9 });
    expect([edited.capacityTable, edited.capacities]).toEqual([9, null]);
    expect(shownRows(edited, { capacities: [capacities[1]!] }).capacities).toEqual([capacities[1]]);
  });
});
