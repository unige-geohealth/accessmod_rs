import { useState } from "react";
import { Columns } from "#ui/components/am/columns";
import { DataTable } from "#ui/components/am/data-table";
import { CheckboxField, RadioField, SelectField, TextField } from "#ui/components/am/form";
import { AmButton, InfoList, LinkActions, MessageList } from "#ui/components/am/messages";
import { PanelAccordion } from "#ui/components/am/panel-accordion";
import { CenterTitle, SectionCard, SubTitle } from "#ui/components/am/section";
import { type SectionTab, SectionTabs } from "#ui/components/am/section-tabs";
import { facilities, fields, layers, scenarioSelected, scenarioToProcess, zonalLayers, zonalTable } from "../mock";

type Module = "accessibility" | "coverage" | "referral" | "zonal" | "scalingUp" | "optimization";

const tabs: SectionTab<Module>[] = [
  { value: "accessibility", label: "analysis_accessibility_main_tab", icon: "accessibility" },
  { value: "coverage", label: "analysis_coverage_main_tab", icon: "coverage" },
  { value: "referral", label: "analysis_referral_main_tab", icon: "referral" },
  { value: "zonal", label: "analysis_zonal_stats_main_tab", icon: "zonal" },
  { value: "scalingUp", label: "analysis_scalingup_main_tab", icon: "scalingUp" },
  { value: "optimization", label: "analysis_optimization_main_tab", icon: "optimization" },
];

const scenarioColumns = ["class", "label", "speed", "mode"].map((field) => ({ field, editable: field !== "class" }));
const facilityColumns = ["cat", "amOnBarrier", "amOnZero", "amCatLandCover", "amPopCell", "name"].map((field) => ({ field }));

/** Analysis section: the same left column for every module, varying fields. */
export function AnalysisScreen({ initial = "accessibility" }: { initial?: Module }) {
  const [module, setModule] = useState<Module>(initial);
  const scaling = module === "scalingUp";
  const [sel, setSel] = useState<Record<string, string>>({
    pop: layers.population[0].value,
    res: layers.populationResidual[0].value,
    lc: layers.landCoverMerged[0].value,
    sc: layers.scenario[0].value,
    hf: layers.facilities[0].value,
    id: "cat",
    name: "name",
    cap: "capacity",
    capTable: layers.capacity[0].value,
    excl: layers.exclusion[0].value,
  });
  const set = (k: string) => (v: string) => setSel({ ...sel, [k]: v });
  const [type, setType] = useState("anisotropic");
  const [dir, setDir] = useState("to");
  const [knight, setKnight] = useState(false);

  if (module === "zonal") {
    return (
      <div className="flex flex-col gap-5">
        <SectionTabs tabs={tabs} value={module} onChange={setModule} />
        <ZonalStatistics />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-5">
      <SectionTabs tabs={tabs} value={module} onChange={setModule} />
      <Columns
        left={
          <>
            <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
              {(scaling || module === "coverage") && (
                <SelectField label="analysis_select_pop_raster" options={layers.population} value={sel.pop} onChange={set("pop")} />
              )}
              {scaling && (
                <SelectField label="analysis_select_residual_pop_raster" options={layers.populationResidual} value={sel.res} onChange={set("res")} />
              )}
              <SelectField label="analysis_select_merged_lc_raster" options={layers.landCoverMerged} value={sel.lc} onChange={set("lc")} />
              <SelectField label="analysis_select_scenario_table" options={layers.scenario} value={sel.sc} onChange={set("sc")} />
              <SelectField label="analysis_select_health_facility_vector" options={layers.facilities} value={sel.hf} onChange={set("hf")} />
              {scaling && (
                <>
                  <SelectField indent label="analysis_select_health_facility_id_field" options={fields.id} value={sel.id} onChange={set("id")} />
                  <SelectField indent label="analysis_select_health_facility_name_field" options={fields.name} value={sel.name} onChange={set("name")} />
                  <SelectField indent label="analysis_select_health_facility_capacity" options={fields.capacity} value={sel.cap} onChange={set("cap")} />
                  <SelectField label="analysis_select_capacity_table" options={layers.capacity} value={sel.capTable} onChange={set("capTable")} />
                  <SelectField label="analysis_select_suitability_table" options={[]} />
                  <SelectField label="analysis_select_exclusion_table" options={layers.exclusion} value={sel.excl} onChange={set("excl")} />
                </>
              )}
            </SectionCard>

            {scaling && (
              <SectionCard title="analysis_scaleup_table" sub="analysis_scaleup_table_sub" icon="tables">
                <PanelAccordion
                  items={[
                    { value: "suit", title: "analysis_scaleup_suitability_index_title", content: <SelectField label="analysis_select_pop_raster" options={layers.population} /> },
                    { value: "excl", title: "analysis_scaleup_exclusion_main", content: <SelectField label="analysis_select_exclusion_table" options={layers.exclusion} /> },
                  ]}
                />
              </SectionCard>
            )}

            <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
              <RadioField
                label="analysis_settings_type"
                value={type}
                onChange={setType}
                options={[
                  { value: "isotropic", label: "analysis_settings_isotropic" },
                  { value: "anisotropic", label: "analysis_settings_anisotropic" },
                ]}
              />
              <RadioField
                label="analysis_settings_travel_direction"
                value={dir}
                onChange={setDir}
                options={[
                  { value: "from", label: "analysis_settings_travel_from_hf" },
                  { value: "to", label: "analysis_settings_travel_to_hf" },
                ]}
              />
              <CheckboxField label="analysis_settings_knight_move" help="analysis_settings_knight_move_desc" checked={knight} onChange={setKnight} />
              <TextField label="analysis_settings_max_travel_time" type="number" value={120} />
              <TextField label="analysis_settings_add_tag" />
            </SectionCard>

            <SectionCard title="analysis_validation_title" sub="analysis_validation_sub" icon="validation">
              <MessageList
                messages={[
                  { kind: "issue", k: "srv_analysis_accessibility_add_tag_instruction" },
                  { kind: "info", k: "srv_analysis_access_unmatched_scenario_labels_warning" },
                ]}
              />
              <AmButton variant="danger" block disabled k="analysis_validation_compute_btn" />
            </SectionCard>
          </>
        }
        right={
          <>
            <CenterTitle title="analysis_travel_scenario_title" sub="analysis_travel_scenario_sub" />
            <div className="px-[15px]">
              <SubTitle k="analysis_travel_scenario_processed" />
              <LinkActions
                actions={[
                  { icon: "reset", k: "analysis_travel_scenario_reset" },
                  { icon: "edit", k: "analysis_travel_scenario_import_selected_table" },
                  { icon: "help", label: "Help" },
                ]}
              />
              <DataTable columns={scenarioColumns} data={scenarioToProcess} height="300px" />
              <SubTitle k="analysis_travel_scenario_selected_table" />
              <DataTable columns={scenarioColumns} data={scenarioSelected} height="300px" />
              <InfoList title="srv_analysis_accessibility_validation_information" items={["srv_analysis_access_unmatched_scenario_labels_warning"]} />
            </div>
            <CenterTitle title="analysis_facility_selection_title" sub="analysis_facility_selection_sub" />
            <div className="px-[15px]">
              <SubTitle k="analysis_facility_selected_facilities" />
              <DataTable columns={facilityColumns} data={facilities} height="420px" selectable />
            </div>
          </>
        }
      />
    </div>
  );
}

const zonalColumns = ["cat", "admin_name", "popTotal", "popTravelTime_1351", "popCoveredPercent_1351"].map((field) => ({ field }));

/** Zonal statistics: travel-time list input, Update button, wide result table with export bar. */
function ZonalStatistics() {
  const [times, setTimes] = useState("1351");
  const [wide, setWide] = useState(true);
  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <SelectField label="analysis_select_pop_raster" options={layers.population} value={layers.population[0].value} />
            <SelectField label="analysis_select_travel_time_raster" options={zonalLayers.travelTime} value={zonalLayers.travelTime[0].value} />
            <SelectField label="analysis_select_zone_vector" options={zonalLayers.zones} value={zonalLayers.zones[0].value} />
            <SelectField indent label="analysis_select_zone_id_integer" options={zonalLayers.zoneId} value="cat" />
            <SelectField indent label="analysis_select_zone_name_text" options={zonalLayers.zoneName} value="admin_name" />
            <TextField label="analysis_select_max_travel_time" help="analysis_select_max_travel_time_desc" value={times} onChange={setTimes} />
          </SectionCard>
          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <CheckboxField
              label="analysis_zonal_stat_check_table_wide"
              help="analysis_zonal_stat_check_table_wide_desc"
              checked={wide}
              onChange={setWide}
            />
          </SectionCard>
          <SectionCard title="analysis_validation_title" sub="analysis_validation_sub" icon="validation">
            <AmButton block k="analysis_validation_update_btn" />
          </SectionCard>
        </>
      }
      right={
        <>
          <CenterTitle title="analysis_zonal_stat_title" sub="analysis_zonal_stat_sub" />
          <LinkActions actions={[{ icon: "help", label: "Help" }]} />
          <DataTable columns={zonalColumns} data={zonalTable} layout="fitData" height="70vh" exportName="am5_zonal_stat" />
        </>
      }
    />
  );
}
