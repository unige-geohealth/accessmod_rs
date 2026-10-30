import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { buildJob, EMPTY, type ReferralDraft, updateReferral } from "./referral";
import { initialRows } from "./scenario";
import type { Context } from "./travel";

const datasets = new Map([1, 2, 3, 4].map((id) => [id, { id, file: `datasets/${id}.x` } as Api.Dataset]));
const scenario = initialRows([1], new Map()).map((r) => ({ ...r, speed: 4 }));
const context: Context = { datasets, dem: 1, scenario };
const facilities = (layer: number) => ({ layer, idField: "cat", selection: { mode: "all" as const, where: "" } });
const ready: ReferralDraft = { ...EMPTY, landCover: 2, from: facilities(3), to: facilities(4), tags: "ref" };

describe("referral draft", () => {
  it("lists what is missing, as AM5, once per message", () => {
    expect(buildJob(EMPTY, context).issues).toEqual([
      "srv_analysis_accessibility_missing_merged_lc_warning",
      "srv_analysis_accessibility_missing_facility_layer",
      "srv_analysis_accessibility_add_tag_instruction",
    ]);
    const none = { mode: "ids" as const, where: "", ids: [] };
    const empty = { ...ready, from: { ...ready.from, selection: none }, to: { ...ready.to, selection: none } };
    expect(buildJob(empty, context).issues).toEqual([
      "srv_analysis_accessibility_select_one_facility_from",
      "srv_analysis_accessibility_select_one_facility_to",
    ]);
  });

  it("builds the referral job with every default explicit; 0 minutes is no limit", () => {
    const draft = updateReferral(ready, { type: "minutes", text: "0" });
    expect(buildJob(draft, context).document).toEqual({
      schema_version: 1,
      dem: "datasets/1.x",
      land_cover: "datasets/2.x",
      scenario: [{ class: 1, label: "no_label_1", speed_kmh: 4, mode: "walking" }],
      method: "anisotropic",
      mobility_model: "accessmod5-v1",
      neighbourhood: 8,
      facility_issues: "error",
      origins: { path: "datasets/3.x", id_field: "cat" },
      destinations: { path: "datasets/4.x", id_field: "cat" },
      limit_s: null,
      nearest_only: false,
      search_from: "origins",
      self_pairs: false,
    });
  });

  it("edits each side alone; permute and closest pair exclude each other", () => {
    const edited = updateReferral(ready, { type: "facilities", side: "to", message: { type: "selection", selection: { mode: "ids", where: "", ids: [7] } } });
    expect(edited.from).toBe(ready.from);
    expect(buildJob(edited, context).document?.destinations).toEqual({ path: "datasets/4.x", id_field: "cat", ids: [7] });
    const closest = updateReferral(updateReferral(ready, { type: "permute", on: true }), { type: "nearestOnly", on: true });
    expect([closest.permute, closest.nearestOnly]).toEqual([false, true]);
    const permuted = updateReferral(closest, { type: "permute", on: true });
    expect(buildJob(permuted, context).document).toMatchObject({ nearest_only: false, search_from: "auto" });
  });
});
