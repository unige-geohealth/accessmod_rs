// Analysis › Accessibility: the draft the screen edits, its update, and the
// travel-time job it makes. Dataset ids refer to the catalogue; the job
// takes their project paths. Derived values (scenario rows of a land cover,
// issues, the job) are selectors, never stored.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import { tagsOf } from "./catalogue";
import { type FacilityInput, type FacilityMessage, facilityIssues, NO_FACILITIES, toFacilitySource, updateFacilities } from "./facility-selection";
import { type Context, fileOf, isTravelMessage, limitSeconds, TRAVEL, type TravelDraft, type TravelMessage, travelJob, updateTravel } from "./travel";

export const ANALYSIS = "travel-time";

export interface AccessibilityDraft extends TravelDraft {
  facilities: FacilityInput;
}

export const EMPTY: AccessibilityDraft = { ...TRAVEL, facilities: NO_FACILITIES };

export type AccessibilityMessage = TravelMessage | { type: "facilities"; message: FacilityMessage };

export function updateAccessibility(draft: AccessibilityDraft, message: AccessibilityMessage): AccessibilityDraft {
  if (isTravelMessage(message)) return updateTravel(draft, message);
  const facilities = updateFacilities(draft.facilities, message.message);
  return facilities === draft.facilities ? draft : { ...draft, facilities };
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
  const travel = travelJob(draft, context);
  const facilities = fileOf(context, draft.facilities.layer);
  const limit = limitSeconds(draft.minutes);
  const { tags, issues: tagIssues } = tagsOf(draft.tags);
  const issues: Key[] = [
    ...travel.landCoverIssues,
    ...travel.scenarioIssues,
    ...facilityIssues(draft.facilities, facilities),
    ...(limit === undefined ? ["srv_analysis_accessibility_max_travel_time_input" as const] : []),
    ...tagIssues,
  ];
  if (issues.length > 0 || travel.fields === undefined || facilities === undefined || limit === undefined) {
    return { document: null, issues, tags };
  }
  return {
    document: {
      schema_version: 1,
      ...travel.fields,
      facilities: toFacilitySource(facilities, draft.facilities.idField, draft.facilities.selection),
      direction: draft.direction,
      ...(limit === null ? {} : { limit_s: limit }),
      outputs: { direction: false },
    },
    issues,
    tags,
  };
}
