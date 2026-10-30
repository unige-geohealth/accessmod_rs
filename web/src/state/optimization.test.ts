import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { buildJob, EMPTY, type OptimizationDraft, positiveInteger, updateOptimization } from "./optimization";

const datasets = new Map([1, 2, 3, 4].map((id) => [id, { id, file: `datasets/${id}.x` } as Api.Dataset]));
const all = { mode: "all" as const, where: "" };
const ready: OptimizationDraft = { ...EMPTY, catchments: { layer: 1, idField: "cat", selection: all }, population: 2, tags: "opt" };

describe("optimization draft", () => {
  it("lists what is missing, as AM5", () => {
    expect(buildJob({ ...EMPTY, count: "0" }, { datasets }).issues).toEqual([
      "validation_m7_no_catchment",
      "validation_m7_no_population",
      "validation_m7_invalid_n_tot",
      "srv_analysis_accessibility_add_tag_instruction",
    ]);
    expect(buildJob({ ...ready, adminCheck: true, perZone: "x" }, { datasets }).issues).toEqual([
      "validation_m7_no_hf_for_admin_check",
      "validation_m7_no_zone_for_admin_check",
      "validation_m7_invalid_np_admin",
    ]);
  });

  it("builds the optimization job, with a zone quota when asked", () => {
    expect(buildJob(ready, { datasets }).document).toEqual({
      schema_version: 1,
      catchments: { path: "datasets/1.x", id_field: "cat" },
      population: "datasets/2.x",
      max_facilities: 10,
      zone_quota: null,
    });
    let draft = ready;
    for (const message of [
      { type: "adminCheck", on: true },
      { type: "facilities", message: { type: "layer", id: 3 } },
      { type: "facilities", message: { type: "idField", field: "cat" } },
      { type: "zones", message: { type: "layer", id: 4 } },
      { type: "zones", message: { type: "idField", field: "cat" } },
      { type: "zones", message: { type: "labelField", field: "name" } },
      { type: "perZone", text: "2" },
      { type: "count", text: "12" },
    ] as const) {
      draft = updateOptimization(draft, message);
    }
    expect(buildJob(draft, { datasets }).document).toMatchObject({
      max_facilities: 12,
      zone_quota: {
        facilities: { path: "datasets/3.x", id_field: "cat" },
        zones: { path: "datasets/4.x", id_field: "cat", label_field: "name" },
        min_per_zone: 2,
      },
    });
  });

  it("reads whole numbers above 0", () => {
    expect(["3", " 4 "].map(positiveInteger)).toEqual([3, 4]);
    for (const bad of ["", "0", "-1", "1.5", "x"]) expect(positiveInteger(bad)).toBeUndefined();
  });
});
