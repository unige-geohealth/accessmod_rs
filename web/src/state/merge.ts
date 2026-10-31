// Toolbox › Merge land cover (AM5 `amAnalysisMergeLandCover`): the stack
// the user builds tab by tab (land cover with its labels, road classes,
// barriers), its order, and the merge-land-cover job it makes. AM5 writes
// each stack item as a raster; here the stack is the draft and the job
// lists its items, so nothing is computed before Merge.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";
import { fileOf, normalizeTags, pickField } from "./catalogue";
import type { Issue } from "./drafts";

export const ANALYSIS = "merge-land-cover";

export interface ClassLabel {
  class: number;
  label: string;
}

/** One item of the stack; `key` identifies it (adding it again replaces it, as AM5 overwrites a stack map). */
export type StackItem =
  | { key: string; type: "land_cover"; dataset: number; labels: readonly ClassLabel[] }
  | { key: string; type: "road"; dataset: number; classField: string; labelField: string; class: number; label: string }
  | { key: string; type: "barrier"; dataset: number };

export interface MergeDraft {
  /** Land cover tab: the raster, its label table, the edited labels (`null`: as read). */
  landCover: number | null;
  labelTable: number | null;
  labels: readonly ClassLabel[] | null;
  /** Roads tab. */
  roads: number | null;
  classField: string;
  labelField: string;
  /** Barriers tab. */
  barrier: number | null;
  /** Merge tab: items used, in priority order (first wins), and skipped. */
  use: readonly StackItem[];
  skip: readonly StackItem[];
  clean: boolean;
  tags: string;
}

export const EMPTY: MergeDraft = {
  landCover: null,
  labelTable: null,
  labels: null,
  roads: null,
  classField: "",
  labelField: "",
  barrier: null,
  use: [],
  skip: [],
  clean: true,
  tags: "",
};

export type MergeMessage =
  | { type: "landCover"; id: number | null }
  | { type: "labelTable"; id: number | null }
  | { type: "labels"; labels: readonly ClassLabel[] }
  | { type: "resetLabels" }
  | { type: "roads"; id: number | null }
  | { type: "classField"; field: string }
  | { type: "labelField"; field: string }
  | { type: "barrier"; id: number | null }
  | { type: "add"; items: readonly StackItem[] }
  | { type: "order"; use: readonly StackItem[]; skip: readonly StackItem[] }
  | { type: "skipAll" }
  | { type: "useAll" }
  | { type: "deleteSkipped" }
  | { type: "clean"; on: boolean }
  | { type: "tags"; text: string };

/** Where an added item goes: roads and barriers above, the land cover below (AM5 default priority). */
function add(draft: MergeDraft, items: readonly StackItem[]): MergeDraft {
  if (items.length === 0) return draft;
  const replace = (list: readonly StackItem[]) => list.map((i) => items.find((n) => n.key === i.key) ?? i);
  const present = new Set([...draft.use, ...draft.skip].map((i) => i.key));
  const fresh = items.filter((i) => !present.has(i.key));
  const top = fresh.filter((i) => i.type !== "land_cover");
  const bottom = fresh.filter((i) => i.type === "land_cover");
  return { ...draft, use: [...top, ...replace(draft.use), ...bottom], skip: replace(draft.skip) };
}

export function updateMerge(draft: MergeDraft, message: MergeMessage): MergeDraft {
  switch (message.type) {
    case "landCover":
      // Labels belong to the classes of a land cover.
      return message.id === draft.landCover ? draft : { ...draft, landCover: message.id, labelTable: null, labels: null };
    case "labelTable":
      return message.id === draft.labelTable ? draft : { ...draft, labelTable: message.id, labels: null };
    case "labels":
      return { ...draft, labels: message.labels };
    case "resetLabels":
      return { ...draft, labels: null };
    case "roads":
      return message.id === draft.roads ? draft : { ...draft, roads: message.id, classField: "", labelField: "" };
    case "classField":
      return { ...draft, classField: message.field };
    case "labelField":
      return { ...draft, labelField: message.field };
    case "barrier":
      return { ...draft, barrier: message.id };
    case "add":
      return add(draft, message.items);
    case "order":
      return { ...draft, use: message.use, skip: message.skip };
    case "skipAll":
      return { ...draft, use: [], skip: [...draft.use, ...draft.skip] };
    case "useAll":
      return { ...draft, use: [...draft.use, ...draft.skip], skip: [] };
    case "deleteSkipped":
      return { ...draft, skip: [] };
    case "clean":
      return { ...draft, clean: message.on };
    case "tags":
      return { ...draft, tags: message.text };
  }
}

/** The draft as shown: unchosen layers and fields take their defaults (AM5: the first layer). */
export function withMergeDefaults(
  draft: MergeDraft,
  first: { landCover: number | null; labelTable: number | null; roads: number | null; barrier: number | null },
  fieldsOf: (layer: number | null) => readonly string[],
): MergeDraft {
  const landCover = draft.landCover ?? first.landCover;
  const labelTable = draft.labelTable ?? first.labelTable;
  const roads = draft.roads ?? first.roads;
  const barrier = draft.barrier ?? first.barrier;
  const fields = fieldsOf(roads);
  const classField = pickField(draft.classField, fields, ["class", "class_n", "cat"]);
  const labelField = pickField(draft.labelField, fields, ["label", "name"]);
  const same =
    landCover === draft.landCover &&
    labelTable === draft.labelTable &&
    roads === draft.roads &&
    barrier === draft.barrier &&
    classField === draft.classField &&
    labelField === draft.labelField;
  return same ? draft : { ...draft, landCover, labelTable, roads, barrier, classField, labelField };
}

/** Labels of a land cover's classes: the table's, or "" (AM5 shows "-", to be filled). */
export const initialLabels = (classes: readonly number[], table: ReadonlyMap<number, string>): ClassLabel[] =>
  classes.map((c) => ({ class: c, label: table.get(c) ?? "" }));

/** Labels shown: the edited ones for the classes of the land cover, else the initial ones. */
export function shownLabels(draft: MergeDraft, initial: readonly ClassLabel[]): readonly ClassLabel[] {
  if (draft.labels === null) return initial;
  const edited = new Map(draft.labels.map((l) => [l.class, l.label]));
  return initial.map((l) => ({ class: l.class, label: edited.get(l.class) ?? l.label }));
}

const norm = (s: string) => s.replace(/\s/g, "").toLowerCase();

/** Land cover tab (AM5): a raster, every class labelled, no label twice. */
export function landCoverIssues(landCover: number | null, labels: readonly ClassLabel[]): Key[] {
  if (landCover === null) return ["srv_merge_landcover_land_cover_missing"];
  if (labels.some((l) => norm(l.label) === "" || norm(l.label) === "-")) return ["srv_merge_landcover_table_missing_or_empty"];
  const seen = new Set<string>();
  for (const l of labels) {
    if (seen.has(norm(l.label))) return ["srv_merge_landcover_table_duplicated_values"];
    seen.add(norm(l.label));
  }
  return [];
}

/** Distinct classes of a road layer, as `GET …/values` lists them. */
export interface RoadClasses {
  values: readonly { value: unknown; label?: string | null }[];
  complete: boolean;
}

/** Roads tab (AM5): a layer, integer classes, each with a label. */
export function roadIssues(roads: number | null, classes: RoadClasses | undefined): Key[] {
  if (roads === null) return ["srv_merge_landcover_road_not_found"];
  if (classes === undefined) return [];
  if (!classes.complete || classes.values.some((v) => typeof v.value !== "number" || !Number.isInteger(v.value))) {
    return ["srv_merge_landcover_road_no_integer_column"];
  }
  if (classes.values.some((v) => (v.label ?? "").trim() === "")) return ["srv_merge_landcover_table_empty_values"];
  return [];
}

/** The classes of a road layer as stack items, one per class (AM5). */
export function roadItems(dataset: number, classField: string, labelField: string, classes: RoadClasses): StackItem[] {
  return classes.values.map((v) => {
    const cls = Number(v.value);
    return { key: `road:${dataset}:${classField}:${cls}`, type: "road", dataset, classField, labelField, class: cls, label: v.label ?? "" };
  });
}

export const landCoverItem = (dataset: number, labels: readonly ClassLabel[]): StackItem => ({
  key: `land_cover:${dataset}`,
  type: "land_cover",
  dataset,
  labels,
});

export const barrierItem = (dataset: number): StackItem => ({ key: `barrier:${dataset}`, type: "barrier", dataset });

/** Class written in the merged land cover by a road class (AM5 adds 1000 below 1000). */
export const roadClass = (cls: number) => (cls < 1000 ? cls + 1000 : cls);

/** A class used by several items of the stack: AM5 "Conflicting classes among items in the stack". */
export interface Conflict {
  item: StackItem;
  class: number;
  label: string;
}

/** Classes of the used items that more than one item writes (barriers write none). */
export function conflicts(use: readonly StackItem[]): Conflict[] {
  const rows: Conflict[] = use.flatMap((item): Conflict[] => {
    switch (item.type) {
      case "land_cover":
        return item.labels.map((l) => ({ item, class: l.class, label: l.label }));
      case "road":
        return [{ item, class: roadClass(item.class), label: item.label }];
      case "barrier":
        return [];
    }
  });
  const count = new Map<number, number>();
  for (const r of rows) count.set(r.class, (count.get(r.class) ?? 0) + 1);
  return rows.filter((r) => (count.get(r.class) ?? 0) > 1).sort((a, b) => a.class - b.class);
}

export interface Built {
  document: Omit<Jobs.MergeLandCoverJob, "output_dir"> | null;
  issues: Issue[];
  tags: string[];
}

/** Merge tab: the job of the used items, or the AM5 issues (one at a time, as AM5). */
export function buildJob(draft: MergeDraft, context: { datasets: ReadonlyMap<number, Api.Dataset>; dem: number | null }): Built {
  const tags = normalizeTags(draft.tags);
  const found = conflicts(draft.use);
  const issues: Issue[] = [];
  if (draft.use.some((i) => fileOf(context, i.dataset) === undefined)) issues.push("srv_merge_landcover_stack_not_found_relaunch");
  else if (found.length > 0) {
    const duplicates = found.length - new Set(found.map((c) => c.class)).size;
    issues.push({ key: "srv_merge_landcover_class_conflicts_found", args: [duplicates] });
  } else if (draft.use.filter((i) => i.type === "land_cover").length !== 1) issues.push("srv_merge_landcover_land_cover_item_required");
  else if (tags.length === 0) issues.push("srv_merge_landcover_tag_required");
  const dem = fileOf(context, context.dem);
  if (issues.length > 0 || dem === undefined) return { document: null, issues, tags };
  const path = (id: number) => fileOf(context, id) ?? "";
  // Adjacent classes of one road layer make one item, their order kept.
  const stack: Jobs.StackItem[] = [];
  let previous: StackItem | undefined;
  for (const item of draft.use) {
    const last = stack.at(-1);
    if (
      item.type === "road" &&
      previous?.type === "road" &&
      last?.type === "road" &&
      previous.dataset === item.dataset &&
      previous.classField === item.classField &&
      previous.labelField === item.labelField
    ) {
      last.classes = [...(last.classes ?? []), item.class];
    } else {
      switch (item.type) {
        case "land_cover":
          stack.push({ type: "land_cover", path: path(item.dataset), labels: item.labels.map((l) => ({ class: l.class, label: l.label })) });
          break;
        case "road":
          stack.push({
            type: "road",
            path: path(item.dataset),
            class_field: item.classField,
            label_field: item.labelField || null,
            classes: [item.class],
          });
          break;
        case "barrier":
          stack.push({ type: "barrier", path: path(item.dataset) });
          break;
      }
    }
    previous = item;
  }
  return { document: { schema_version: 1, dem, stack, clean_bridges: draft.clean }, issues, tags };
}
