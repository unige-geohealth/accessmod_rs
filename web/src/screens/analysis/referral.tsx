// Analysis › Referral (AM5 module 4): travel time and distance from each
// facility of the From group to the facilities of the To group.
import { Columns } from "@accessmod/ui/am/columns";
import { CheckboxField } from "@accessmod/ui/am/form";
import { SectionCard } from "@accessmod/ui/am/section";
import { T, useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import schema from "../../../schemas/jobs/referral.json";
import { datasetOptions, fieldClasses } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import { withFacilityDefaults } from "../../state/facility-selection";
import { ANALYSIS, buildJob, EMPTY, type ReferralDraft, type Side, updateReferral } from "../../state/referral";
import { hasZero } from "../../state/scenario";
import { useStore } from "../../state/store";
import { FacilityInputs, FacilitySection } from "./facility-section";
import { fieldsOf, firstOf, useCatalogue, TagsField } from "./inputs";
import { LimitField, ScenarioSection, TravelInputs, TravelSettings, useTravel } from "./travel";
import { ValidationCard } from "./validation-card";

const LAND_COVER = fieldClasses(schema, "land_cover");
const FACILITIES = fieldClasses(schema, "origins");

export function Referral({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updateReferral);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId, dem } = useCatalogue(project);
  const travel = useTravel(project, stored, LAND_COVER);
  const facilityOptions = datasetOptions(t, datasets, FACILITIES);
  const fields = (id: number | null) => fieldsOf(byId, id);
  const from = withFacilityDefaults(stored.from, firstOf(facilityOptions), fields);
  const to = withFacilityDefaults(stored.to, firstOf(facilityOptions), fields);
  const draft: ReferralDraft = from === stored.from && to === stored.to ? travel.draft : { ...travel.draft, from, to };
  const { scenario } = travel;
  const built = useMemo(() => buildJob(draft, { datasets: byId, dem, scenario }), [draft, byId, dem, scenario]);
  const group = (side: Side) => (side === "from" ? from : to);

  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <TravelInputs travel={travel} dispatch={dispatch} />
            {(["from", "to"] as const).map((side) => (
              <div key={side} className="flex flex-col gap-[15px]">
                <p className="font-bold">
                  <T k={side === "from" ? "analysis_select_from" : "analysis_select_to"} />
                </p>
                <FacilityInputs
                  label={side === "from" ? "analysis_select_health_facility_vector" : "analysis_select_existing_health_facility_vector"}
                  idLabel={side === "from" ? "analysis_select_health_facility_id_field" : "analysis_select_health_facility_id_field_to"}
                  options={facilityOptions}
                  input={group(side)}
                  fields={fields(group(side).layer)}
                  onMessage={(message) => dispatch({ type: "facilities", side, message })}
                />
              </div>
            ))}
          </SectionCard>

          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <TravelSettings draft={draft} dispatch={dispatch} direction={false} />
            <CheckboxField
              label="analysis_settings_referral_permute_groups"
              help="analysis_settings_referral_permute_groups_desc"
              checked={draft.permute}
              onChange={(on) => dispatch({ type: "permute", on })}
            />
            <CheckboxField
              label="analysis_settings_referral_limit_closest"
              help="analysis_settings_referral_limit_closest_desc"
              checked={draft.nearestOnly}
              onChange={(on) => dispatch({ type: "nearestOnly", on })}
            />
            <LimitField draft={draft} dispatch={dispatch} />
            <TagsField project={project} analysis={ANALYSIS} value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
          </SectionCard>

          <ValidationCard
            project={project}
            analysis={ANALYSIS}
            issues={built.issues}
            notices={hasZero(scenario) ? ["srv_analysis_accessibility_tbl_speed_has_zero"] : []}
            document={built.document}
            tags={built.tags}
            datasets={byId}
          />
        </>
      }
      right={
        <>
          <ScenarioSection project={project} travel={travel} dispatch={dispatch} />
          {(["from", "to"] as const).map((side) => (
            <FacilitySection
              key={side}
              project={project}
              input={group(side)}
              landCover={draft.landCover}
              scenario={scenario}
              onMessage={(message) => dispatch({ type: "facilities", side, message })}
              group={side === "from" ? "analysis_facility_selection_from" : "analysis_facility_selection_to"}
              titled={side === "from"}
            />
          ))}
        </>
      }
    />
  );
}
