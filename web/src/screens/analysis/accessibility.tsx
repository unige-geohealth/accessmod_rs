// Analysis › Accessibility (AM5 module 2): travel time from or towards the
// facilities.
import { Columns } from "@accessmod/ui/am/columns";
import { TextField } from "@accessmod/ui/am/form";
import { SectionCard } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import travelTimeSchema from "../../../schemas/jobs/travel-time.json";
import { ANALYSIS, buildJob, EMPTY, updateAccessibility } from "../../state/accessibility";
import { datasetOptions, fieldClasses } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import { type FacilityMessage, withFacilityDefaults } from "../../state/facility-selection";
import { hasZero } from "../../state/scenario";
import { useStore } from "../../state/store";
import { FacilityInputs, FacilitySection } from "./facility-section";
import { fieldsOf, firstOf, useCatalogue } from "./inputs";
import { ScenarioSection, TravelInputs, TravelSettings, useTravel } from "./travel";
import { ValidationCard } from "./validation-card";

const LAND_COVER = fieldClasses(travelTimeSchema, "land_cover");
const FACILITIES = fieldClasses(travelTimeSchema, "facilities");

export function Accessibility({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updateAccessibility);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId, dem } = useCatalogue(project);
  const travel = useTravel(project, stored, LAND_COVER);
  const facilityOptions = datasetOptions(t, datasets, FACILITIES);
  const facilities = withFacilityDefaults(stored.facilities, firstOf(facilityOptions), (id) => fieldsOf(byId, id));
  const draft = facilities === stored.facilities ? travel.draft : { ...travel.draft, facilities };
  const { scenario } = travel;
  const built = useMemo(() => buildJob(draft, { datasets: byId, dem, scenario }), [draft, byId, dem, scenario]);
  const onFacilities = (message: FacilityMessage) => dispatch({ type: "facilities", message });

  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <TravelInputs travel={travel} dispatch={dispatch} />
            <FacilityInputs
              label="analysis_select_health_facility_vector"
              idLabel="analysis_select_health_facility_id_field"
              options={facilityOptions}
              input={facilities}
              fields={fieldsOf(byId, facilities.layer)}
              onMessage={onFacilities}
            />
          </SectionCard>

          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <TravelSettings draft={draft} dispatch={dispatch} />
            <TextField label="analysis_settings_add_tag" value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
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
          <FacilitySection project={project} input={facilities} landCover={draft.landCover} scenario={scenario} onMessage={onFacilities} />
        </>
      }
    />
  );
}
