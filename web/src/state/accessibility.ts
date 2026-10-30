// Analysis › Accessibility: the draft the screen edits, its update, and the
// travel-time job it makes. Dataset ids refer to the catalogue; the job
// takes their project paths. Derived values (scenario rows of a land cover,
// issues, the job) are selectors, never stored.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";
import { normalizeTags } from "./catalogue";
import { ALL, type FacilitySelection, toFacilitySource } from "./facility-selection";
import { mergeTable, type ScenarioRow, toScenario } from "./scenario";

export const ANALYSIS = "travel-time";

/** AM5 default maximum travel time, minutes; 0: no limit. */
export const DEFAULT_MINUTES = 120;

export interface AccessibilityDraft {
  landCover: number | null;
  scenarioTable: number | null;
  facilities: number | null;
  /** Identifier field of the facilities; "" until chosen. */
  idField: string;
  /** Edited scenario rows; `null`: the initial rows of the land cover. */
  scenario: readonly ScenarioRow[] | null;
  selection: FacilitySelection;
  method: Jobs.MethodName;
  direction: Jobs.Direction;
  knight: boolean;
  /** Maximum travel time as typed, minutes. */
  minutes: string;
  tags: string;
}

export const EMPTY: AccessibilityDraft = {
  landCover: null,
  scenarioTable: null,
  facilities: null,
  idField: "",
  scenario: null,
  selection: ALL,
  method: "anisotropic",
  direction: "from_facilities",
  knight: false,
  minutes: String(DEFAULT_MINUTES),
  tags: "",
};

export type AccessibilityMessage =
  | { type: "landCover"; id: number | null }
  | { type: "scenarioTable"; id: number | null }
  | { type: "facilities"; id: number | null }
  | { type: "idField"; field: string }
  | { type: "scenario"; rows: readonly ScenarioRow[] }
  | { type: "resetScenario" }
  | { type: "selection"; selection: FacilitySelection }
  | { type: "method"; method: Jobs.MethodName }
  | { type: "direction"; direction: Jobs.Direction }
  | { type: "knight"; on: boolean }
  | { type: "minutes"; text: string }
  | { type: "tags"; text: string }
  | { type: "load"; draft: AccessibilityDraft };

export function updateAccessibility(draft: AccessibilityDraft, message: AccessibilityMessage): AccessibilityDraft {
  switch (message.type) {
    case "landCover":
      // Scenario rows belong to the classes of a land cover.
      return message.id === draft.landCover ? draft : { ...draft, landCover: message.id, scenario: null };
    case "scenarioTable":
      return { ...draft, scenarioTable: message.id };
    case "facilities":
      // Fields and identifiers belong to a layer.
      return message.id === draft.facilities ? draft : { ...draft, facilities: message.id, idField: "", selection: ALL };
    case "idField":
      return message.field === draft.idField ? draft : { ...draft, idField: message.field, selection: { mode: "all", where: draft.selection.where } };
    case "scenario":
      return { ...draft, scenario: message.rows };
    case "resetScenario":
      return { ...draft, scenario: null };
    case "selection":
      return { ...draft, selection: message.selection };
    case "method":
      return { ...draft, method: message.method };
    case "direction":
      return { ...draft, direction: message.direction };
    case "knight":
      return { ...draft, knight: message.on };
    case "minutes":
      return { ...draft, minutes: message.text };
    case "tags":
      return { ...draft, tags: message.text };
    case "load":
      return message.draft;
  }
}

/** Scenario rows shown: the land cover's initial rows, with the draft's edits. */
export function scenarioRows(draft: AccessibilityDraft, initial: readonly ScenarioRow[]): readonly ScenarioRow[] {
  return draft.scenario === null ? initial : mergeTable(initial, draft.scenario);
}

/** Largest maximum travel time AM5 accepts, minutes. */
const MAX_MINUTES = 2 ** 31 - 1;

/** Maximum travel time in seconds (`null`: no limit), or `undefined` when invalid. */
export function limitSeconds(text: string): number | null | undefined {
  const minutes = Number(text.trim());
  if (text.trim() === "" || !Number.isFinite(minutes) || minutes < 0 || minutes > MAX_MINUTES) return undefined;
  return minutes === 0 ? null : minutes * 60;
}

/** What the job needs from the catalogue. */
export interface Context {
  datasets: ReadonlyMap<number, Api.Dataset>;
  /** DEM dataset of the project. */
  dem: number | null;
  /** Scenario rows shown (edited, or initial of the land cover). */
  scenario: readonly ScenarioRow[];
}

export interface Built {
  /** Job document, paths relative to the project; `null` while incomplete. */
  document: Omit<Jobs.TravelTimeJob, "output_dir"> | null;
  /** Blocking issues, as AM5 lists them. */
  issues: Key[];
  tags: string[];
}

/** The job of a draft, or what is missing (AM5 validation messages). */
export function buildJob(draft: AccessibilityDraft, context: Context): Built {
  const issues: Key[] = [];
  const file = (id: number | null) => (id === null ? undefined : context.datasets.get(id)?.file);
  const landCover = file(draft.landCover);
  const facilities = file(draft.facilities);
  const dem = file(context.dem);
  if (landCover === undefined) issues.push("srv_analysis_accessibility_missing_merged_lc_warning");
  if (facilities === undefined) issues.push("srv_analysis_accessibility_missing_facility_layer");
  if (facilities !== undefined && draft.idField === "") issues.push("analysis.id_field_missing");
  const scenario = toScenario(context.scenario);
  if (landCover !== undefined && !scenario.some((e) => e.speed_kmh > 0)) issues.push("analysis.scenario_no_speed");
  if (draft.selection.mode === "ids" && draft.selection.ids.length === 0) issues.push("srv_analysis_accessibility_select_facilities");
  const limit = limitSeconds(draft.minutes);
  if (limit === undefined) issues.push("srv_analysis_accessibility_max_travel_time_input");
  const tags = normalizeTags(draft.tags);
  if (tags.length === 0) issues.push("srv_analysis_accessibility_add_tag_instruction");
  if (issues.length > 0 || landCover === undefined || facilities === undefined || dem === undefined || limit === undefined) {
    return { document: null, issues, tags };
  }
  return {
    document: {
      schema_version: 1,
      dem,
      land_cover: landCover,
      facilities: toFacilitySource(facilities, draft.idField, draft.selection),
      scenario,
      method: draft.method,
      mobility_model: "accessmod5-v1",
      direction: draft.direction,
      neighbourhood: draft.knight ? 16 : 8,
      ...(limit === null ? {} : { limit_s: limit }),
      facility_issues: "error",
      outputs: { direction: false },
    },
    issues,
    tags,
  };
}

/** What the screen shows before the user chooses: AM5 preselects the first layer of each class and the `cat` field. */
export interface Defaults {
  landCover: number | null;
  facilities: number | null;
  /** Fields of a facility layer, to pick the default identifier from. */
  fieldsOf: (facilities: number | null) => readonly string[];
}

/** The draft as shown: unchosen inputs take their defaults. */
export function withDefaults(draft: AccessibilityDraft, defaults: Defaults): AccessibilityDraft {
  const landCover = draft.landCover ?? defaults.landCover;
  const facilities = draft.facilities ?? defaults.facilities;
  const fields = defaults.fieldsOf(facilities);
  const idField = draft.idField !== "" && fields.includes(draft.idField) ? draft.idField : fields.includes("cat") ? "cat" : (fields[0] ?? "");
  if (landCover === draft.landCover && facilities === draft.facilities && idField === draft.idField) return draft;
  return { ...draft, landCover, facilities, idField };
}
