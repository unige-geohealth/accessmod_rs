// Toolbox › Adjust population distribution (AM5 `amPopulationBarrierCorrection`):
// population moved off barriers, within zones, optionally scaled to the
// known population of each zone.
import { RadioField } from "@accessmod/ui/am/form";
import { CenterTitle, SectionCard } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import schema from "../../../schemas/jobs/adjust-population-distribution.json";
import { datasetOptions, fieldClasses } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import { ANALYSIS, buildJob, EMPTY, updatePopulation, withPopulationDefaults } from "../../state/population";
import { useStore } from "../../state/store";
import { DatasetSelect, FieldSelect, fieldsOf, firstOf, TagsField, useCatalogue } from "../analysis/inputs";
import { ValidationCard } from "../analysis/validation-card";

const POPULATION = fieldClasses(schema, "population");
const LAND_COVER = fieldClasses(schema, "land_cover");
const ZONES = fieldClasses(schema, "zones");

export function AdjustPopulation({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updatePopulation);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId, dem } = useCatalogue(project);
  const options = {
    population: datasetOptions(t, datasets, POPULATION),
    landCover: datasetOptions(t, datasets, LAND_COVER),
    zones: datasetOptions(t, datasets, ZONES),
  };
  const fields = (id: number | null) => fieldsOf(byId, id);
  const draft = withPopulationDefaults(
    stored,
    { population: firstOf(options.population), landCover: firstOf(options.landCover), zones: firstOf(options.zones) },
    fields,
  );
  const built = useMemo(() => buildJob(draft, { datasets: byId, dem }), [draft, byId, dem]);

  return (
    <>
      <CenterTitle title="toolbox_popcor_main_title" sub="toolbox_popcor_sub" />
      <div className="grid grid-cols-12 gap-x-[30px]">
        <div className="col-span-12 flex flex-col gap-5 md:col-span-4">
          <SectionCard title="toolbox_popcor_config_title" sub="toolbox_popcor_config_sub">
            <DatasetSelect
              label="toolbox_popcor_select_pop_raster"
              options={options.population}
              value={draft.population}
              onChange={(id) => dispatch({ type: "population", id })}
            />
            <DatasetSelect
              label="toolbox_popcor_select_merged_lc_raster"
              options={options.landCover}
              value={draft.landCover}
              onChange={(id) => dispatch({ type: "landCover", id })}
            />
            <hr className="border-separator" />
            <RadioField
              label="toolbox_popcor_mode"
              value={draft.known ? "known" : "unknown"}
              onChange={(v) => dispatch({ type: "known", on: v === "known" })}
              options={[
                { value: "known", label: "toolbox_popcor_mode_pop_known" },
                { value: "unknown", label: "toolbox_popcor_mode_pop_not_known" },
              ]}
            />
            <DatasetSelect
              label="toolbox_popcor_select_zones_vector"
              options={options.zones}
              value={draft.zones}
              onChange={(id) => dispatch({ type: "zones", id })}
            />
            <FieldSelect
              label="analysis_select_zone_id_integer"
              fields={fields(draft.zones)}
              value={draft.idField}
              onChange={(field) => dispatch({ type: "idField", field })}
            />
            {draft.known && (
              <FieldSelect
                label="toolbox_popcor_select_zones_pop"
                fields={fields(draft.zones)}
                value={draft.populationField}
                onChange={(field) => dispatch({ type: "populationField", field })}
              />
            )}
            <hr className="border-separator" />
            <TagsField project={project} analysis={ANALYSIS} label="toolbox_popcor_add_tag" value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
          </SectionCard>
          <ValidationCard
            project={project}
            analysis={ANALYSIS}
            issues={built.issues}
            notices={[]}
            document={built.document}
            tags={built.tags}
            datasets={byId}
            button="toolbox_popcor_compute_btn"
          />
        </div>
      </div>
    </>
  );
}
