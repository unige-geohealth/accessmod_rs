// Analysis › Geographic coverage (AM5 module 3): facilities, in a processing
// order, take the population they reach up to their capacity.
import { Columns } from "@accessmod/ui/am/columns";
import { CheckboxField, Field, RadioField, TextField } from "@accessmod/ui/am/form";
import { SectionCard } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import schema from "../../../schemas/jobs/geographic-coverage.json";
import { datasetOptions, fieldClasses } from "../../state/catalogue";
import { ANALYSIS, buildJob, type CoverageDraft, type CoverageField, type CoverageOption, EMPTY, updateCoverage, withFieldDefaults } from "../../state/coverage";
import { draftStore } from "../../state/drafts";
import { type FacilityMessage, withFacilityDefaults } from "../../state/facility-selection";
import { hasZero } from "../../state/scenario";
import { useStore } from "../../state/store";
import { withZoneDefaults } from "../../state/zones";
import { FacilityInputs, FacilitySection } from "./facility-section";
import { DatasetSelect, FieldSelect, fieldsOf, firstOf, useCatalogue, ZoneInputs } from "./inputs";
import { LimitField, ScenarioSection, TagsField, TravelInputs, TravelSettings, useTravel } from "./travel";
import { ValidationCard } from "./validation-card";

const LAND_COVER = fieldClasses(schema, "land_cover");
const POPULATION = fieldClasses(schema, "population");
const FACILITIES = fieldClasses(schema, "facilities");
const ZONES = fieldClasses(schema, "zones");

/** AM5 `mod3param`, in its order. */
const OPTIONS: [CoverageOption, Parameters<typeof CheckboxField>[0]["label"]][] = [
  ["catchments", "analysis_settings_options_compute_catchment"],
  ["removeCovered", "analysis_settings_options_remove_covered_population"],
  ["populationOnBarriers", "analysis_settings_options_compute_population_on_barrier"],
  ["zonal", "analysis_settings_options_compute_zonal_statistic"],
  ["ignoreCapacity", "analysis_settings_options_ignore_capacity"],
];

export function Coverage({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updateCoverage);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId, dem } = useCatalogue(project);
  const travel = useTravel(project, stored, LAND_COVER);
  const populationOptions = datasetOptions(t, datasets, POPULATION);
  const facilityOptions = datasetOptions(t, datasets, FACILITIES);
  const zoneOptions = datasetOptions(t, datasets, ZONES);
  const fields = (id: number | null) => fieldsOf(byId, id);
  const facilities = withFacilityDefaults(stored.facilities, firstOf(facilityOptions), fields);
  const zones = withZoneDefaults(stored.zones, firstOf(zoneOptions), fields);
  const shown: CoverageDraft = { ...travel.draft, population: stored.population ?? firstOf(populationOptions), facilities, zones };
  const draft = withFieldDefaults(shown, fields(facilities.layer));
  const { scenario } = travel;
  const built = useMemo(() => buildJob(draft, { datasets: byId, dem, scenario }), [draft, byId, dem, scenario]);
  const onFacilities = (message: FacilityMessage) => dispatch({ type: "facilities", message });
  const field = (name: CoverageField) => (value: string) => dispatch({ type: "field", field: name, value });
  const facilityFields = fields(facilities.layer);

  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <DatasetSelect
              label="analysis_select_pop_raster"
              options={populationOptions}
              value={draft.population}
              onChange={(id) => dispatch({ type: "population", id })}
            />
            <TravelInputs travel={travel} dispatch={dispatch} />
            <FacilityInputs
              label="analysis_select_health_facility_vector"
              idLabel="analysis_select_health_facility_id_field"
              options={facilityOptions}
              input={facilities}
              fields={facilityFields}
              onMessage={onFacilities}
            />
            <FieldSelect label="analysis_select_health_facility_name_field" fields={facilityFields} value={draft.nameField} onChange={field("nameField")} />
            {!draft.ignoreCapacity && (
              <FieldSelect label="analysis_select_health_facility_capacity" fields={facilityFields} value={draft.capacityField} onChange={field("capacityField")} />
            )}
            {draft.zonal && (
              <ZoneInputs options={zoneOptions} input={zones} fields={fields(zones.layer)} onMessage={(message) => dispatch({ type: "zones", message })} />
            )}
          </SectionCard>

          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <TravelSettings draft={draft} dispatch={dispatch} />
            <RadioField
              label="analysis_settings_health_facility_order"
              value={draft.order}
              onChange={(v) => dispatch({ type: "order", order: v as CoverageDraft["order"] })}
              options={[
                { value: "field", label: "analysis_settings_health_facility_order_table" },
                { value: "population", label: "analysis_settings_health_facility_order_travel_time" },
              ]}
            />
            {draft.order === "field" ? (
              <FieldSelect label="analysis_settings_health_facility_select" fields={facilityFields} value={draft.orderField} onChange={field("orderField")} />
            ) : (
              <TextField
                label="analysis_settings_given_travel_time"
                type="number"
                value={draft.orderMinutes}
                onChange={(text) => dispatch({ type: "orderMinutes", text })}
              />
            )}
            <RadioField
              label="analysis_settings_sorting_health_facility"
              value={draft.descending ? "desc" : "asc"}
              onChange={(v) => dispatch({ type: "option", option: "descending", on: v === "desc" })}
              options={[
                { value: "asc", label: "analysis_settings_sorting_health_facility_asc" },
                { value: "desc", label: "analysis_settings_sorting_health_facility_desc" },
              ]}
            />
            <LimitField draft={draft} dispatch={dispatch} />
            <Field label="analysis_settings_options">
              {OPTIONS.map(([option, label]) => (
                <CheckboxField key={option} label={label} checked={draft[option]} onChange={(on) => dispatch({ type: "option", option, on })} />
              ))}
            </Field>
            <TagsField value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
          </SectionCard>

          <ValidationCard
            project={project}
            analysis={ANALYSIS}
            issues={built.issues}
            notices={[...built.notices, ...(hasZero(scenario) ? ["srv_analysis_accessibility_tbl_speed_has_zero" as const] : [])]}
            document={built.document}
            tags={built.tags}
            datasets={byId}
          />
        </>
      }
      right={
        <>
          <ScenarioSection project={project} travel={travel} dispatch={dispatch} />
          <FacilitySection
            project={project}
            input={facilities}
            landCover={draft.landCover}
            population={draft.population}
            scenario={scenario}
            onMessage={onFacilities}
          />
        </>
      }
    />
  );
}
