import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { buildJob, type CoverageDraft, EMPTY, UNLIMITED_S, updateCoverage, withFieldDefaults } from "./coverage";
import { initialRows } from "./scenario";
import type { Context } from "./travel";

const datasets = new Map([1, 2, 3, 4, 5].map((id) => [id, { id, file: `datasets/${id}.x` } as Api.Dataset]));
const scenario = initialRows([1], new Map()).map((r) => ({ ...r, speed: 4 }));
const context: Context = { datasets, dem: 1, scenario };
const ready: CoverageDraft = {
  ...EMPTY,
  landCover: 2,
  population: 3,
  facilities: { layer: 4, idField: "cat", selection: { mode: "all", where: "" } },
  nameField: "name",
  capacityField: "capacity",
  orderField: "capacity",
  tags: "cov",
};

describe("geographic coverage draft", () => {
  it("lists what is missing, as AM5", () => {
    expect(buildJob({ ...EMPTY, minutes: "x" }, context).issues).toEqual([
      "srv_analysis_accessibility_missing_merged_lc_warning",
      "srv_analysis_accessibility_missing_facility_layer",
      "srv_analysis_accessibility_max_travel_time_input",
      "srv_analysis_accessibility_add_tag_instruction",
      "srv_analysis_accessibility_select_population",
    ]);
    const noCapacity = buildJob({ ...ready, capacityField: "", zonal: true }, context);
    expect(noCapacity.issues).toEqual(["srv_analysis_accessibility_set_capacity_warning", "srv_analysis_accessibility_select_zone_warning"]);
    expect(buildJob({ ...ready, capacityField: "", ignoreCapacity: true }, context).issues).toEqual([]);
  });

  it("builds the geographic-coverage job with every default explicit", () => {
    const built = buildJob(ready, context);
    expect(built.notices).toEqual([]);
    expect(built.document).toEqual({
      schema_version: 1,
      dem: "datasets/1.x",
      land_cover: "datasets/2.x",
      scenario: [{ class: 1, label: "no_label_1", speed_kmh: 4, mode: "walking" }],
      method: "anisotropic",
      mobility_model: "accessmod5-v1",
      neighbourhood: 8,
      facility_issues: "error",
      population: "datasets/3.x",
      facilities: { path: "datasets/4.x", id_field: "cat" },
      name_field: "name",
      capacity_field: "capacity",
      direction: "from_facilities",
      limit_s: 7200,
      band_s: 60,
      order: { field: { field: "capacity", descending: true } },
      catchments: true,
      remove_covered: true,
      population_on_barriers: true,
      ignore_capacity: false,
      zones: null,
    });
  });

  it("orders by population reached, uses zones, and reads 0 minutes as AM5's 32 767", () => {
    let draft = ready;
    for (const message of [
      { type: "order", order: "population" },
      { type: "orderMinutes", text: "30" },
      { type: "option", option: "descending", on: false },
      { type: "option", option: "removeCovered", on: false },
      { type: "option", option: "zonal", on: true },
      { type: "zones", message: { type: "layer", id: 5 } },
      { type: "zones", message: { type: "idField", field: "cat" } },
      { type: "zones", message: { type: "labelField", field: "name" } },
      { type: "minutes", text: "0" },
    ] as const) {
      draft = updateCoverage(draft, message);
    }
    const built = buildJob(draft, context);
    expect(built.notices).toEqual(["srv_analysis_accessibility_max_travel_time_set_0min", "srv_analysis_accessibility_facilities_processing_order"]);
    expect(built.document).toMatchObject({
      limit_s: UNLIMITED_S,
      order: { population: { limit_s: 1800, descending: false } },
      remove_covered: false,
      zones: { path: "datasets/5.x", id_field: "cat", label_field: "name" },
    });
  });

  it("resets the facility fields with the layer, and defaults them by name", () => {
    const other = updateCoverage(ready, { type: "facilities", message: { type: "layer", id: 5 } });
    expect([other.nameField, other.capacityField, other.orderField]).toEqual(["", "", ""]);
    expect(withFieldDefaults(other, ["cat", "name", "capacity"])).toMatchObject({ nameField: "name", capacityField: "capacity", orderField: "capacity" });
    expect(withFieldDefaults(ready, ["cat", "name", "capacity"])).toBe(ready);
  });
});
