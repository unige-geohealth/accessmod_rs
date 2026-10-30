// What every travel-time analysis edits (Accessibility, Geographic coverage,
// Referral, Scaling up): land cover, travel scenario, method, maximum travel
// time and tags. A tab's draft extends `TravelDraft`; its update hands the
// shared messages to `updateTravel`.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";
import { fileOf } from "./catalogue";
import { mergeTable, type ScenarioRow, toScenario } from "./scenario";

/** AM5 default maximum travel time, minutes; 0: no limit. */
export const DEFAULT_MINUTES = 120;

export interface TravelDraft {
  landCover: number | null;
  scenarioTable: number | null;
  /** Edited scenario rows; `null`: the initial rows of the land cover. */
  scenario: readonly ScenarioRow[] | null;
  method: Jobs.MethodName;
  direction: Jobs.Direction;
  knight: boolean;
  /** Maximum travel time as typed, minutes. */
  minutes: string;
  tags: string;
}

export const TRAVEL: TravelDraft = {
  landCover: null,
  scenarioTable: null,
  scenario: null,
  method: "anisotropic",
  direction: "from_facilities",
  knight: false,
  minutes: String(DEFAULT_MINUTES),
  tags: "",
};

export type TravelMessage =
  | { type: "landCover"; id: number | null }
  | { type: "scenarioTable"; id: number | null }
  | { type: "scenario"; rows: readonly ScenarioRow[] }
  | { type: "resetScenario" }
  | { type: "method"; method: Jobs.MethodName }
  | { type: "direction"; direction: Jobs.Direction }
  | { type: "knight"; on: boolean }
  | { type: "minutes"; text: string }
  | { type: "tags"; text: string };

const TYPES: ReadonlySet<string> = new Set<TravelMessage["type"]>([
  "landCover",
  "scenarioTable",
  "scenario",
  "resetScenario",
  "method",
  "direction",
  "knight",
  "minutes",
  "tags",
]);

export const isTravelMessage = (message: { type: string }): message is TravelMessage => TYPES.has(message.type);

export function updateTravel<D extends TravelDraft>(draft: D, message: TravelMessage): D {
  switch (message.type) {
    case "landCover":
      // Scenario rows belong to the classes of a land cover.
      return message.id === draft.landCover ? draft : { ...draft, landCover: message.id, scenario: null };
    case "scenarioTable":
      return { ...draft, scenarioTable: message.id };
    case "scenario":
      return { ...draft, scenario: message.rows };
    case "resetScenario":
      return { ...draft, scenario: null };
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
  }
}

/** Scenario rows shown: the land cover's initial rows, with the draft's edits. */
export function scenarioRows(draft: TravelDraft, initial: readonly ScenarioRow[]): readonly ScenarioRow[] {
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

/** What a job needs from the catalogue. */
export interface Context {
  datasets: ReadonlyMap<number, Api.Dataset>;
  /** DEM dataset of the project. */
  dem: number | null;
  /** Scenario rows shown (edited, or initial of the land cover). */
  scenario: readonly ScenarioRow[];
}


/** The travel part of a job, or `undefined` while incomplete. */
export interface Travel {
  fields:
    | {
        dem: string;
        land_cover: string;
        scenario: Jobs.ScenarioEntry[];
        method: Jobs.MethodName;
        mobility_model: "accessmod5-v1";
        neighbourhood: 8 | 16;
        facility_issues: "error";
      }
    | undefined;
  /** Land cover and scenario issues, as AM5 lists them. */
  landCoverIssues: Key[];
  scenarioIssues: Key[];
}

export function travelJob(draft: TravelDraft, context: Context): Travel {
  const landCover = fileOf(context, draft.landCover);
  const dem = fileOf(context, context.dem);
  const scenario = toScenario(context.scenario);
  const landCoverIssues: Key[] = landCover === undefined ? ["srv_analysis_accessibility_missing_merged_lc_warning"] : [];
  const scenarioIssues: Key[] = landCover !== undefined && !scenario.some((e) => e.speed_kmh > 0) ? ["analysis.scenario_no_speed"] : [];
  const complete = landCover !== undefined && dem !== undefined && scenarioIssues.length === 0;
  return {
    fields: complete
      ? {
          dem,
          land_cover: landCover,
          scenario,
          method: draft.method,
          mobility_model: "accessmod5-v1",
          neighbourhood: draft.knight ? 16 : 8,
          facility_issues: "error",
        }
      : undefined,
    landCoverIssues,
    scenarioIssues,
  };
}

/** The draft as shown: an unchosen land cover takes its default (AM5: the first layer). */
export function withTravelDefaults<D extends TravelDraft>(draft: D, landCover: number | null): D {
  return draft.landCover !== null || landCover === null ? draft : { ...draft, landCover };
}
