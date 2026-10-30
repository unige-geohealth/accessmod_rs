// Analysis › Referral (AM5 module 4): travel time and distance between two
// groups of facilities, From and To.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import { tagsOf } from "./catalogue";
import { type FacilityInput, type FacilityMessage, facilityIssues, NO_FACILITIES, toFacilitySource, updateFacilities } from "./facility-selection";
import { type Context, fileOf, isTravelMessage, limitSeconds, TRAVEL, type TravelDraft, type TravelMessage, travelJob, updateTravel } from "./travel";

export const ANALYSIS = "referral";

export type Side = "from" | "to";

export interface ReferralDraft extends TravelDraft {
  from: FacilityInput;
  to: FacilityInput;
  /** AM5 "Permute groups": search from the smaller side (`search_from: auto`). */
  permute: boolean;
  /** AM5 "Limit the spatial analysis to the closest pair time-wise". */
  nearestOnly: boolean;
}

export const EMPTY: ReferralDraft = { ...TRAVEL, from: NO_FACILITIES, to: NO_FACILITIES, permute: false, nearestOnly: false };

export type ReferralMessage =
  | TravelMessage
  | { type: "facilities"; side: Side; message: FacilityMessage }
  | { type: "permute"; on: boolean }
  | { type: "nearestOnly"; on: boolean };

export function updateReferral(draft: ReferralDraft, message: ReferralMessage): ReferralDraft {
  if (isTravelMessage(message)) return updateTravel(draft, message);
  switch (message.type) {
    case "facilities": {
      const input = updateFacilities(draft[message.side], message.message);
      return input === draft[message.side] ? draft : { ...draft, [message.side]: input };
    }
    // AM5: the two options exclude each other.
    case "permute":
      return { ...draft, permute: message.on, nearestOnly: message.on ? false : draft.nearestOnly };
    case "nearestOnly":
      return { ...draft, nearestOnly: message.on, permute: message.on ? false : draft.permute };
  }
}

export interface Built {
  document: Omit<Jobs.ReferralJob, "output_dir"> | null;
  issues: Key[];
  tags: string[];
}

export function buildJob(draft: ReferralDraft, context: Context): Built {
  const travel = travelJob(draft, context);
  const from = fileOf(context, draft.from.layer);
  const to = fileOf(context, draft.to.layer);
  const limit = limitSeconds(draft.minutes);
  const { tags, issues: tagIssues } = tagsOf(draft.tags);
  const issues: Key[] = [
    ...new Set<Key>([
      ...travel.landCoverIssues,
      ...facilityIssues(draft.from, from, "srv_analysis_accessibility_select_one_facility_from"),
      ...facilityIssues(draft.to, to, "srv_analysis_accessibility_select_one_facility_to"),
    ]),
    ...(limit === undefined ? ["srv_analysis_accessibility_max_travel_time_input" as const] : []),
    ...tagIssues,
    ...travel.scenarioIssues,
  ];
  if (issues.length > 0 || travel.fields === undefined || from === undefined || to === undefined || limit === undefined) {
    return { document: null, issues, tags };
  }
  return {
    document: {
      schema_version: 1,
      ...travel.fields,
      origins: toFacilitySource(from, draft.from.idField, draft.from.selection),
      destinations: toFacilitySource(to, draft.to.idField, draft.to.selection),
      limit_s: limit,
      nearest_only: draft.nearestOnly,
      search_from: draft.permute ? "auto" : "origins",
      self_pairs: false,
    },
    issues,
    tags,
  };
}
