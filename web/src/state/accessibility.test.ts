import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { buildJob, EMPTY, updateAccessibility } from "./accessibility";
import { NO_FACILITIES } from "./facility-selection";
import { initialRows } from "./scenario";
import { type Context, scenarioRows } from "./travel";

const dataset = (id: number, file: string) => ({ id, file }) as Api.Dataset;
const datasets = new Map([1, 2, 3].map((id) => [id, dataset(id, `datasets/${id}.x`)]));
const rows = initialRows([1, 2], new Map());
const context = (scenario: Context["scenario"] = rows): Context => ({ datasets, dem: 1, scenario });

describe("accessibility draft", () => {
  it("lists what is missing, as AM5", () => {
    const built = buildJob({ ...EMPTY, minutes: "-1" }, context());
    expect(built.document).toBeNull();
    expect(built.issues).toEqual([
      "srv_analysis_accessibility_missing_merged_lc_warning",
      "srv_analysis_accessibility_missing_facility_layer",
      "srv_analysis_accessibility_max_travel_time_input",
      "srv_analysis_accessibility_add_tag_instruction",
    ]);
  });

  it("builds the travel-time job", () => {
    let draft = EMPTY;
    for (const message of [
      { type: "landCover", id: 2 },
      { type: "facilities", message: { type: "layer", id: 3 } },
      { type: "facilities", message: { type: "idField", field: "cat" } },
      { type: "scenario", rows: [{ ...rows[0]!, speed: 5 }] },
      { type: "knight", on: true },
      { type: "direction", direction: "towards_facilities" },
      { type: "tags", text: "dry season" },
    ] as const) {
      draft = updateAccessibility(draft, message);
    }
    const shown = scenarioRows(draft, rows);
    expect(shown.map((r) => r.speed)).toEqual([5, 0]);
    const built = buildJob(draft, context(shown));
    expect(built.issues).toEqual([]);
    expect(built.tags).toEqual(["dry", "season"]);
    expect(built.document).toEqual({
      schema_version: 1,
      dem: "datasets/1.x",
      land_cover: "datasets/2.x",
      facilities: { path: "datasets/3.x", id_field: "cat" },
      scenario: [
        { class: 1, label: "no_label_1", speed_kmh: 5, mode: "walking" },
        { class: 2, label: "no_label_2", speed_kmh: 0, mode: "walking" },
      ],
      method: "anisotropic",
      mobility_model: "accessmod5-v1",
      direction: "towards_facilities",
      neighbourhood: 16,
      limit_s: 7200,
      facility_issues: "error",
      outputs: { direction: false },
    });
  });

  it("resets what depends on a changed layer", () => {
    const edited = updateAccessibility(
      { ...EMPTY, landCover: 2, facilities: { layer: 3, idField: "cat", selection: { mode: "ids", where: "x > 1", ids: [1] } }, scenario: rows },
      { type: "landCover", id: 4 },
    );
    expect(edited.scenario).toBeNull();
    const other = updateAccessibility(edited, { type: "facilities", message: { type: "layer", id: 5 } });
    expect(other.facilities).toEqual({ ...NO_FACILITIES, layer: 5 });
  });

  it("requires a speed and a checked facility", () => {
    const facilities = { layer: 3, idField: "cat", selection: { mode: "ids" as const, where: "", ids: [] } };
    const draft = { ...EMPTY, landCover: 2, facilities, tags: "a" };
    expect(buildJob(draft, context()).issues).toEqual(["analysis.scenario_no_speed", "srv_analysis_accessibility_select_facilities"]);
  });
});
