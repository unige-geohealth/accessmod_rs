// Analysis › Scaling up (AM5 module 6): new facilities placed one after the
// other on the most suitable cells.
import { Columns } from "@accessmod/ui/am/columns";
import { RadioField, TextField } from "@accessmod/ui/am/form";
import { SectionCard, SubTitle } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import schema from "../../../schemas/jobs/scaling-up.json";
import { useRows } from "../../api/queries";
import { datasetOptions, fieldClasses } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import { type FacilityMessage, withFacilityDefaults } from "../../state/facility-selection";
import { ANALYSIS, buildJob, EMPTY, type ScalingUpDraft, shownRows, type Table, type TableStatus, updateScalingUp } from "../../state/scaling-up";
import { capacityRows, exclusionRows, factorRows } from "../../state/scaling-up-tables";
import { hasZero } from "../../state/scenario";
import { useStore } from "../../state/store";
import { FacilityInputs, FacilitySection } from "./facility-section";
import { DatasetSelect, fieldsOf, firstOf, useCatalogue, TagsField } from "./inputs";
import { RuleForms, ScalingUpTables } from "./scaling-up-tables";
import { LimitField, ScenarioSection, TravelInputs, TravelSettings, useTravel, WHOLE } from "./travel";
import { ValidationCard } from "./validation-card";

const LAND_COVER = fieldClasses(schema, "land_cover");
const POPULATION = fieldClasses(schema, "population");
const RESIDUAL = fieldClasses(schema, "population_residual");
const FACILITIES = fieldClasses(schema, "facilities");

const TABLES = [
  ["capacities", "capacityTable", "tCapacity", "analysis_select_capacity_table"],
  ["factors", "factorTable", "tSuitability", "analysis_select_suitability_table"],
  ["exclusions", "exclusionTable", "tExclusion", "analysis_select_exclusion_table"],
] as const;

export function ScalingUp({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updateScalingUp);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId, dem } = useCatalogue(project);
  const travel = useTravel(project, stored, LAND_COVER);
  const options = {
    population: datasetOptions(t, datasets, POPULATION),
    residual: datasetOptions(t, datasets, RESIDUAL),
    facilities: datasetOptions(t, datasets, FACILITIES),
    capacities: datasetOptions(t, datasets, ["tCapacity"]),
    factors: datasetOptions(t, datasets, ["tSuitability"]),
    exclusions: datasetOptions(t, datasets, ["tExclusion"]),
  };
  const fields = (id: number | null) => fieldsOf(byId, id);
  const facilities = withFacilityDefaults(stored.facilities, firstOf(options.facilities), fields);
  const draft: ScalingUpDraft = {
    ...travel.draft,
    population: stored.population ?? firstOf(options.population),
    residual: stored.residual ?? firstOf(options.residual),
    facilities,
    capacityTable: stored.capacityTable ?? firstOf(options.capacities),
    factorTable: stored.factorTable ?? firstOf(options.factors),
    exclusionTable: stored.exclusionTable ?? firstOf(options.exclusions),
  };
  const reads = {
    capacities: useRows(project, draft.capacityTable, WHOLE),
    factors: useRows(project, draft.factorTable, WHOLE),
    exclusions: useRows(project, draft.exclusionTable, WHOLE),
  };
  // Only a table read whole and current is used: no rows of a previous selection.
  const usable = (q: (typeof reads)["capacities"]) => (q.isSuccess && !q.isPlaceholderData && q.data.rows.length >= q.data.total ? q.data : undefined);
  const capacityTable = usable(reads.capacities);
  const factorTable = usable(reads.factors);
  const exclusionTable = usable(reads.exclusions);
  const status = Object.fromEntries(
    TABLES.flatMap(([table, field]) => {
      const q = reads[table];
      if (draft[field] === null || usable(q) !== undefined) return [];
      return [[table, q.isSuccess && !q.isPlaceholderData ? "too_large" : "pending"]];
    }),
  ) as Partial<Record<Table, TableStatus>>;
  const tables = useMemo(
    () => ({
      capacities: capacityTable && capacityRows(capacityTable),
      factors: factorTable && factorRows(factorTable, datasets ?? []),
      exclusions: exclusionTable && exclusionRows(exclusionTable, datasets ?? []),
    }),
    [capacityTable, factorTable, exclusionTable, datasets],
  );
  const rows = shownRows(draft, tables);
  const { scenario } = travel;
  const built = buildJob(draft, { datasets: byId, dem, scenario }, rows, status);
  const onFacilities = (message: FacilityMessage) => dispatch({ type: "facilities", message });
  const limit = (name: "coverPercent" | "newFacilities" | "maxMinutes") => (text: string) => dispatch({ type: "limit", limit: name, text });

  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <DatasetSelect label="analysis_select_pop_raster" options={options.population} value={draft.population} onChange={(id) => dispatch({ type: "population", id })} />
            <DatasetSelect
              label="analysis_select_residual_pop_raster"
              options={options.residual}
              value={draft.residual}
              onChange={(id) => dispatch({ type: "residual", id })}
            />
            <TravelInputs travel={travel} dispatch={dispatch} />
            {draft.useExisting && (
              <FacilityInputs
                label="analysis_select_health_facility_vector"
                idLabel="analysis_select_health_facility_id_field"
                options={options.facilities}
                input={facilities}
                fields={fields(facilities.layer)}
                onMessage={onFacilities}
              />
            )}
            {TABLES.map(([table, field, , label]) => (
              <DatasetSelect
                key={table}
                label={label}
                options={options[table]}
                value={draft[field]}
                onChange={(id) => dispatch({ type: "table", table: table as Table, id })}
              />
            ))}
          </SectionCard>

          <SectionCard title="analysis_scaleup_table" sub="analysis_scaleup_table_sub" icon="tables">
            <RuleForms datasets={datasets} factors={rows.factors} exclusions={rows.exclusions} dispatch={dispatch} />
          </SectionCard>

          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <RadioField
              label="analysis_settings_existing_health_facility"
              value={draft.useExisting ? "existing" : "empty"}
              onChange={(v) => dispatch({ type: "useExisting", on: v === "existing" })}
              options={[
                { value: "empty", label: "analysis_settings_use_facility_empty" },
                { value: "existing", label: "analysis_settings_use_facility_existing" },
              ]}
            />
            <SubTitle k="analysis_settings_param_new_health_facility" />
            <TravelSettings draft={draft} dispatch={dispatch} />
            <LimitField draft={draft} dispatch={dispatch} />
            <SubTitle k="analysis_settings_compute_limits" />
            <TextField label="analysis_settings_pop_cover_percent" type="number" value={draft.coverPercent} onChange={limit("coverPercent")} />
            <TextField label="analysis_settings_new_health_facility_number" type="number" value={draft.newFacilities} onChange={limit("newFacilities")} />
            <TextField label="analysis_settings_max_process_time" type="number" value={draft.maxMinutes} onChange={limit("maxMinutes")} />
            <TagsField project={project} analysis={ANALYSIS} value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
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
          <ScalingUpTables capacities={rows.capacities} factors={rows.factors} exclusions={rows.exclusions} byId={byId} dispatch={dispatch} />
          {draft.useExisting && (
            <FacilitySection project={project} input={facilities} landCover={draft.landCover} scenario={scenario} onMessage={onFacilities} />
          )}
        </>
      }
    />
  );
}
