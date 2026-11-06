import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import {
  barrierItem,
  buildJob,
  conflicts,
  EMPTY,
  initialLabels,
  landCoverIssues,
  landCoverItem,
  type MergeDraft,
  roadIssues,
  roadNotices,
  roadItems,
  shownLabels,
  updateMerge,
} from "./merge";

const dataset = (id: number, file: string) => ({ id, file }) as Api.Dataset;
const datasets = new Map([1, 2, 3, 4].map((id) => [id, dataset(id, `datasets/${id}.x`)]));
const context = { datasets, dem: 1 };
const roads = {
  complete: true,
  values: [
    { value: 1, label: "Main road" },
    { value: 2, label: "Secondary" },
  ],
};

describe("merge land cover draft", () => {
  it("checks the labels of the land cover tab, as AM5", () => {
    expect(landCoverIssues(null, [])).toEqual(["srv_merge_landcover_land_cover_missing"]);
    const initial = initialLabels([1, 2], new Map([[1, "forest"]]));
    expect(landCoverIssues(2, initial)).toEqual(["srv_merge_landcover_table_missing_or_empty"]);
    expect(
      landCoverIssues(2, [
        { class: 1, label: "Forest" },
        { class: 2, label: " forest" },
      ]),
    ).toEqual(["srv_merge_landcover_table_duplicated_values"]);
    const edited = updateMerge(EMPTY, { type: "labels", labels: [{ class: 2, label: "crops" }] });
    expect(shownLabels(edited, initial)).toEqual([
      { class: 1, label: "forest" },
      { class: 2, label: "crops" },
    ]);
  });

  it("checks the road classes", () => {
    expect(roadIssues(null, undefined)).toEqual(["srv_merge_landcover_road_not_found"]);
    expect(roadIssues(3, { complete: true, values: [{ value: "a", label: "x" }] })).toEqual(["srv_merge_landcover_road_no_integer_column"]);
    expect(roadIssues(3, { complete: true, values: [{ value: 1, label: "" }] })).toEqual(["srv_merge_landcover_table_empty_values"]);
    expect(roadIssues(3, { complete: true, values: [{ value: 1, label: "-" }] })).toEqual(["srv_merge_landcover_table_empty_values"]);
    const twice = {
      complete: true,
      values: [
        { value: 1, label: "road" },
        { value: 2, label: "Road " },
      ],
    };
    expect(roadIssues(3, twice)).toEqual(["srv_merge_landcover_table_duplicated_values_warning"]);
    const mixed = { complete: true, values: [{ value: 1, label: "main", several_labels: true }] };
    expect(roadIssues(3, mixed)).toEqual(["srv_merge_landcover_table_duplicated_values_warning"]);
    expect(roadIssues(3, roads)).toEqual([]);
    expect(roadNotices(roads)).toEqual(["srv_merge_landcover_classes_under_1000"]);
    expect(roadNotices({ complete: true, values: [{ value: 1001, label: "x" }] })).toEqual([]);
  });

  it("stacks roads and barriers above the land cover; adding again replaces", () => {
    let draft: MergeDraft = updateMerge(EMPTY, { type: "add", items: [landCoverItem(2, [{ class: 1, label: "forest" }])] });
    draft = updateMerge(draft, { type: "add", items: roadItems(3, "class_n", "label", roads) });
    draft = updateMerge(draft, { type: "add", items: [barrierItem(4)] });
    draft = updateMerge(draft, { type: "add", items: [landCoverItem(2, [{ class: 1, label: "woods" }])] });
    expect(draft.use.map((i) => i.key)).toEqual(["barrier:4", "road:3:class_n:1", "road:3:class_n:2", "land_cover:2"]);
    expect(draft.use.at(-1)).toMatchObject({ labels: [{ class: 1, label: "woods" }] });
    const skipped = updateMerge(draft, { type: "skipAll" });
    expect([skipped.use.length, skipped.skip.length]).toEqual([0, 4]);
    expect(updateMerge(skipped, { type: "deleteSkipped" }).skip).toEqual([]);
  });

  it("finds classes written by several items (roads count from 1000)", () => {
    const use = [
      landCoverItem(2, [
        { class: 1001, label: "built" },
        { class: 3, label: "crops" },
      ]),
      ...roadItems(3, "class_n", "label", roads),
    ];
    expect(conflicts(use).map((c) => [c.class, c.label])).toEqual([
      [1001, "built"],
      [1001, "Main road"],
    ]);
    expect(buildJob({ ...EMPTY, use, tags: "merged" }, context).issues).toEqual([{ key: "srv_merge_landcover_class_conflicts_found", args: [1] }]);
  });

  it("needs one land cover and a tag, then builds the job in stack order", () => {
    const roadsOnly = { ...EMPTY, use: roadItems(3, "class_n", "label", roads) };
    expect(buildJob(roadsOnly, context).issues).toEqual(["srv_merge_landcover_land_cover_item_required"]);
    const [main, secondary] = roadItems(3, "class_n", "label", roads);
    const use = [secondary!, main!, barrierItem(4), landCoverItem(2, [{ class: 1, label: "forest" }])];
    expect(buildJob({ ...EMPTY, use }, context).issues).toEqual(["srv_merge_landcover_tag_required"]);
    const built = buildJob({ ...EMPTY, use, tags: "merged", clean: false }, context);
    expect(built.document).toEqual({
      schema_version: 1,
      dem: "datasets/1.x",
      clean_bridges: false,
      stack: [
        // Adjacent classes of a road layer: one item, in stack order.
        { type: "road", path: "datasets/3.x", class_field: "class_n", label_field: "label", classes: [2, 1] },
        { type: "barrier", path: "datasets/4.x" },
        { type: "land_cover", path: "datasets/2.x", labels: [{ class: 1, label: "forest" }] },
      ],
    });
  });

  it("keeps road classes below 1000 when asked (AM5 advanced option)", () => {
    expect(updateMerge(EMPTY, { type: "keepClasses", on: false })).toBe(EMPTY);
    const draft = updateMerge(EMPTY, { type: "keepClasses", on: true });
    expect(draft.keepClasses).toBe(true);
    expect(roadNotices(roads, false)).toEqual([]);
    const [kept] = roadItems(3, "class_n", "label", roads, false);
    const [shifted] = roadItems(3, "class_n", "label", roads);
    // Class 1 kept clashes with land cover class 1; shifted (1001) does not.
    const forest = landCoverItem(2, [{ class: 1, label: "forest" }]);
    expect(conflicts([kept!, forest]).map((c) => c.class)).toEqual([1, 1]);
    expect(conflicts([shifted!, forest])).toEqual([]);
    const built = buildJob({ ...EMPTY, use: [kept!, shifted!, landCoverItem(2, [{ class: 5, label: "forest" }])], tags: "m" }, context);
    // Adding the class again with the other option replaces the stacked item.
    const replaced = updateMerge(updateMerge(EMPTY, { type: "add", items: [kept!] }), { type: "add", items: [shifted!] });
    expect(replaced.use).toEqual([shifted]);
    // Items of one layer with different options are not grouped.
    expect(built.document?.stack.slice(0, 2)).toEqual([
      { type: "road", path: "datasets/3.x", class_field: "class_n", label_field: "label", classes: [1], add_1000: false },
      { type: "road", path: "datasets/3.x", class_field: "class_n", label_field: "label", classes: [1] },
    ]);
  });
});
