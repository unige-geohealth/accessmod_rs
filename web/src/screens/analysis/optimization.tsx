// Analysis › Optimization (AM5 module 7): select the facilities whose
// catchments cover the most population.
import { Columns } from "@accessmod/ui/am/columns";
import { CheckboxField, Field, TextField } from "@accessmod/ui/am/form";
import { SectionCard } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import schema from "../../../schemas/jobs/optimization.json";
import { datasetOptions, fieldClasses } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import { withFacilityDefaults } from "../../state/facility-selection";
import { ANALYSIS, buildJob, EMPTY, type OptimizationDraft, updateOptimization } from "../../state/optimization";
import { useStore } from "../../state/store";
import { withZoneDefaults } from "../../state/zones";
import { FacilityInputs } from "./facility-section";
import { DatasetSelect, fieldsOf, firstOf, useCatalogue, ZoneInputs } from "./inputs";
import { TagsField } from "./travel";
import { ValidationCard } from "./validation-card";

const CATCHMENTS = fieldClasses(schema, "catchments");
const POPULATION = fieldClasses(schema, "population");
const QUOTA = (schema as { $defs: { ZoneQuotaJob: unknown } }).$defs.ZoneQuotaJob;
const FACILITIES = fieldClasses(QUOTA, "facilities");
const ZONES = fieldClasses(QUOTA, "zones");

export function Optimization({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updateOptimization);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId } = useCatalogue(project);
  const options = {
    catchments: datasetOptions(t, datasets, CATCHMENTS),
    population: datasetOptions(t, datasets, POPULATION),
    facilities: datasetOptions(t, datasets, FACILITIES),
    zones: datasetOptions(t, datasets, ZONES),
  };
  const fields = (id: number | null) => fieldsOf(byId, id);
  const draft: OptimizationDraft = {
    ...stored,
    // Catchments of a geographic coverage name their facility in `facility_id`.
    catchments: withFacilityDefaults(stored.catchments, firstOf(options.catchments), fields, ["facility_id", "cat"]),
    population: stored.population ?? firstOf(options.population),
    facilities: withFacilityDefaults(stored.facilities, firstOf(options.facilities), fields),
    zones: withZoneDefaults(stored.zones, firstOf(options.zones), fields),
  };
  const built = useMemo(() => buildJob(draft, { datasets: byId }), [draft, byId]);

  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <FacilityInputs
              label="analysis_select_catchment_layer"
              idLabel="analysis_select_catchment_id_field"
              options={options.catchments}
              input={draft.catchments}
              fields={fields(draft.catchments.layer)}
              onMessage={(message) => dispatch({ type: "catchments", message })}
            />
            <DatasetSelect
              label="analysis_select_pop_raster"
              options={options.population}
              value={draft.population}
              onChange={(id) => dispatch({ type: "population", id })}
            />
            {draft.adminCheck && (
              <>
                <FacilityInputs
                  label="analysis_select_health_facility_vector"
                  idLabel="analysis_select_health_facility_id_field"
                  options={options.facilities}
                  input={draft.facilities}
                  fields={fields(draft.facilities.layer)}
                  onMessage={(message) => dispatch({ type: "facilities", message })}
                />
                <ZoneInputs
                  options={options.zones}
                  input={draft.zones}
                  fields={fields(draft.zones.layer)}
                  onMessage={(message) => dispatch({ type: "zones", message })}
                />
              </>
            )}
          </SectionCard>

          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <TextField label="analysis_optimization_n_tot" type="number" value={draft.count} onChange={(text) => dispatch({ type: "count", text })} />
            <Field label="analysis_optimization_options">
              <CheckboxField label="analysis_optimization_admin_check" checked={draft.adminCheck} onChange={(on) => dispatch({ type: "adminCheck", on })} />
            </Field>
            {draft.adminCheck && (
              <TextField label="analysis_optimization_np_admin" type="number" value={draft.perZone} onChange={(text) => dispatch({ type: "perZone", text })} />
            )}
            <TagsField value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
          </SectionCard>

          <ValidationCard
            project={project}
            analysis={ANALYSIS}
            issues={built.issues}
            notices={[]}
            document={built.document}
            tags={built.tags}
            datasets={byId}
          />
        </>
      }
      right={null}
    />
  );
}
