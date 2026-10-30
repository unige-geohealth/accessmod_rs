import { useState, type ReactNode } from "react";
import { Callout, AmButton, FilePicker, InfoList, LinkActions, MessageList } from "#ui/components/am/messages";
import { ConfirmDialog } from "#ui/components/am/confirm-dialog";
import { CheckboxField, RadioField, SelectField, TextField } from "#ui/components/am/form";
import { DataTable } from "#ui/components/am/data-table";
import { icons } from "#ui/components/am/icons";
import { PanelAccordion } from "#ui/components/am/panel-accordion";
import { CenterTitle, SectionCard } from "#ui/components/am/section";
import { SectionTabs } from "#ui/components/am/section-tabs";
import { MapFrame } from "#ui/components/am/map-frame";
import { PageTabs } from "#ui/components/am/page-tabs";
import { type Stack, StackList } from "#ui/components/am/stack-list";
import { Progress } from "#ui/components/ui/progress";
import { T } from "#ui/i18n";
import { layers, scenarioSelected, stack as stackMock, zonalTable } from "../mock";

const swatches = [
  "background", "foreground", "muted", "muted-foreground", "border", "separator", "primary", "link",
  "sidebar", "sidebar-muted", "sidebar-accent", "well", "well-border", "panel-heading", "panel-border",
  "button", "button-active", "button-border", "danger", "callout", "warning", "table-row-alt", "table-row-selected", "input",
];

function Block({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mb-8">
      <h3 className="mb-3 border-b border-separator pb-1 text-2xl">{title}</h3>
      {children}
    </section>
  );
}

/** Every pattern on one page; check it in light and dark. */
export function Catalogue() {
  const [tab, setTab] = useState<"a" | "b" | "c">("a");
  const [radio, setRadio] = useState("to");
  const [check, setCheck] = useState(true);
  const [stack, setStack] = useState<Stack>({ use: stackMock.skip.slice(0, 2), skip: stackMock.skip.slice(2, 5) });
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState(false);

  return (
    <div className="max-w-5xl">
      <Block title="Tokens">
        <div className="grid grid-cols-6 gap-2">
          {swatches.map((s) => (
            <div key={s} className="text-xs">
              <div className="h-10 border border-border" style={{ background: `var(--${s})` }} />
              <code>--{s}</code>
            </div>
          ))}
        </div>
      </Block>

      <Block title="Typography">
        <CenterTitle title="analysis_travel_scenario_title" sub="analysis_travel_scenario_sub" className="[&_h2]:mt-0" />
        <h3 className="text-2xl">h3 24px — Data input</h3>
        <h4 className="text-lg">h4 18px — Travel scenario to be processed</h4>
        <p>Body 14px Source Sans. <a className="text-link hover:underline" href="#/catalogue">Link</a>. <span className="text-muted-foreground">Muted</span>. <span className="text-warning">Warning</span>.</p>
      </Block>

      <Block title="Section tabs">
        <SectionTabs
          value={tab}
          onChange={setTab}
          tabs={[
            { value: "a", label: "analysis_accessibility_main_tab", icon: "accessibility" },
            { value: "b", label: "analysis_coverage_main_tab", icon: "coverage" },
            { value: "c", label: "analysis_scalingup_main_tab", icon: "scalingUp" },
          ]}
        />
      </Block>

      <Block title="Buttons">
        <div className="flex flex-wrap gap-2">
          <AmButton k="data_choose_import_btn" />
          <AmButton icon="archive" k="logs_download" />
          <AmButton variant="primary" k="project_import_btn" />
          <AmButton variant="danger" k="analysis_validation_compute_btn" />
          <AmButton variant="danger" disabled k="analysis_validation_compute_btn" />
          <FilePicker k="project_import_btn" accept=".tif" onFiles={() => {}} />
          <AmButton variant="danger" k="data_delete_permanently" onClick={() => setConfirming(true)} />
          <ConfirmDialog open={confirming} onOpenChange={setConfirming} confirm="data_delete_permanently" onConfirm={() => {}}>
            <T k="data_delete_warning" />
          </ConfirmDialog>
        </div>
      </Block>

      <div className="grid grid-cols-2 gap-[30px]">
        <div>
          <Block title="Section card + form">
            <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
              <SelectField label="analysis_select_merged_lc_raster" options={layers.landCoverMerged} value={layers.landCoverMerged[0].value} />
              <SelectField indent label="analysis_select_health_facility_id_field" options={[{ value: "cat", label: "cat" }]} value="cat" />
              <SelectField label="analysis_select_suitability_table" options={[]} />
              <RadioField
                label="analysis_settings_travel_direction"
                value={radio}
                onChange={setRadio}
                options={[
                  { value: "from", label: "analysis_settings_travel_from_hf" },
                  { value: "to", label: "analysis_settings_travel_to_hf" },
                ]}
              />
              <CheckboxField label="analysis_settings_knight_move" help="analysis_settings_knight_move_desc" checked={check} onChange={setCheck} />
              <TextField label="analysis_settings_max_travel_time" type="number" value={120} />
            </SectionCard>
          </Block>
        </div>
        <div>
          <Block title="Accordion">
            <PanelAccordion
              defaultValue="import"
              items={[
                { value: "import", icon: "add", title: "data_import_data", content: <InfoList title="srv_analysis_accessibility_validation_information" items={["srv_data_automatic_name"]} /> },
                { value: "filter", icon: "filter", title: "data_filter_data", content: "…" },
                { value: "delete", icon: "delete", title: "data_delete_data", content: "…" },
              ]}
            />
          </Block>
          <Block title="Messages">
            <Callout k="analysis_zonal_stat_add" />
            <MessageList
              messages={[
                { kind: "issue", k: "srv_analysis_accessibility_add_tag_instruction" },
                { kind: "info", k: "srv_analysis_access_unmatched_scenario_labels_warning" },
              ]}
            />
          </Block>
          <Block title="Progress">
            <Progress value={42} />
          </Block>
        </div>
      </div>

      <Block title="Table">
        <LinkActions
          actions={[
            { icon: "reset", k: "analysis_travel_scenario_reset" },
            { icon: "edit", k: "analysis_travel_scenario_import_selected_table" },
            { icon: "help", label: "Help" },
          ]}
        />
        <DataTable selectable columns={["class", "label", "speed", "mode"].map((field) => ({ field, editable: true }))} data={scenarioSelected} height="260px" />
      </Block>

      <Block title="Table with export bar (fitData)">
        <DataTable columns={Object.keys(zonalTable[0]).map((field) => ({ field }))} data={zonalTable} layout="fitData" height="180px" exportName="am5_zonal_stat" />
      </Block>

      <Block title="Page tabs">
        <PageTabs
          tabs={[
            { value: "lc", label: "toolbox_land_cover_tab", content: "…" },
            { value: "roads", label: "toolbox_land_cover_roads_tab", content: "…" },
            { value: "merge", label: "toolbox_land_cover_merge_tab", content: "…" },
          ]}
        />
      </Block>

      <Block title="Stack list">
        <StackList value={stack} onChange={setStack} />
      </Block>

      <Block title="Map (offline basemap, edit controls)">
        <MapFrame
          className="h-[360px]"
          bounds={[33.9, -16.6, 35.9, -14.2]}
          editing={{ active: editing, onToggle: () => setEditing(!editing), onNext: () => {}, canUndo: true }}
        />
      </Block>

      <Block title="Icons">
        <div className="grid grid-cols-6 gap-3">
          {Object.entries(icons).map(([name, Icon]) => (
            <div key={name} className="flex items-center gap-2 text-xs">
              <Icon className="size-4" />
              {name}
            </div>
          ))}
        </div>
      </Block>
    </div>
  );
}
