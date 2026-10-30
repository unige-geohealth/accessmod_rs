import { useState } from "react";
import { Slider } from "#ui/components/ui/slider";
import { Columns } from "#ui/components/am/columns";
import { DataTable } from "#ui/components/am/data-table";
import { CheckboxField, Field, RadioField, SelectField, TextField } from "#ui/components/am/form";
import { MapFrame } from "#ui/components/am/map-frame";
import { AmButton, InfoList } from "#ui/components/am/messages";
import { PanelAccordion } from "#ui/components/am/panel-accordion";
import { CenterTitle } from "#ui/components/am/section";
import { T } from "#ui/i18n";
import { dataList, layers, logs } from "../mock";

const projects = ["446_skeleton", "demo", "ethiopia_final", "yaniss_guinea"].map((p) => ({ value: p, label: p }));
const languageOptions = [
  { value: "en", label: "English" },
  { value: "fr", label: "Français" },
  { value: "es", label: "Español" },
  { value: "pt", label: "Português" },
];

export function ProjectScreen() {
  return (
    <Columns
      left={
        <PanelAccordion
          defaultValue="open"
          items={[
            { value: "open", icon: "open", title: "project_open_title", content: <SelectField label="project_open_select" options={projects} value="demo" /> },
            {
              value: "new",
              icon: "add",
              title: "project_new_import_title",
              content: (
                <div className="flex flex-col gap-[15px]">
                  <TextField label="project_new_name_text" />
                  <AmButton block k="project_import_btn" />
                </div>
              ),
            },
            {
              value: "delete",
              icon: "delete",
              title: "project_delete_title",
              content: (
                <div className="flex flex-col gap-[15px]">
                  <SelectField label="project_delete_select_project" options={projects} />
                  <AmButton variant="danger" block k="project_delete_btn" />
                </div>
              ),
            },
            { value: "export", icon: "export", title: "project_export_title", content: <AmButton block k="project_export_btn" /> },
            { value: "language", icon: "language", title: "project_language_title", content: <SelectField label="project_language_select" options={languageOptions} value="en" /> },
          ]}
        />
      }
      right={
        <>
          <CenterTitle title="project_summary_title" sub="project_summary_sub" />
          <PanelAccordion
            defaultValue="map"
            items={[
              { value: "map", title: "project_location_map_title", content: <MapFrame className="h-[400px]" bounds={[33.9, -16.6, 35.9, -14.2]} /> },
              { value: "proj", title: "project_projection_system_title", content: <pre className="text-xs">EPSG:32736 — WGS 84 / UTM zone 36S</pre> },
              { value: "grid", title: "project_grid_parameter_title", content: <pre className="text-xs">rows 1650 · cols 1172 · res 100 m</pre> },
              { value: "extm", title: "project_extent_metric_title", content: <pre className="text-xs">N 8473000 · S 8308000 · E 721000 · W 603800</pre> },
              { value: "extd", title: "project_extent_degrees_title", content: <pre className="text-xs">N -13.8 · S -15.3 · E 35.0 · W 33.9</pre> },
            ]}
          />
        </>
      }
      wide
    />
  );
}

export function DataScreen() {
  const [cls, setCls] = useState(layers.dataClasses[0].value);
  return (
    <Columns
      wide
      left={
        <PanelAccordion
          defaultValue="import"
          items={[
            {
              value: "import",
              icon: "add",
              title: "data_import_data",
              content: (
                <div className="flex flex-col gap-[15px]">
                  <SelectField label="data_select_data_class" options={layers.dataClasses} value={cls} onChange={setCls} />
                  <TextField label="data_add_tag" />
                  <InfoList items={["srv_data_import_new_dem_warning", "srv_data_automatic_name"]} />
                  <AmButton block k="data_choose_import_btn" />
                </div>
              ),
            },
            { value: "filter", icon: "filter", title: "data_filter_data", content: <TextField label="data_add_tag" /> },
            { value: "rename", icon: "rename", title: "data_rename_data", content: <TextField label="data_add_tag" /> },
            { value: "archive", icon: "archive", title: "data_archive_data", content: <TextField label="data_add_tag" /> },
            { value: "delete", icon: "delete", title: "data_delete_data", content: <TextField label="data_add_tag" /> },
          ]}
        />
      }
      right={
        <>
          <CenterTitle title="data_available_title" sub="data_available_sub" />
          <DataTable selectable columns={[{ field: "tags", title: "Tags" }, { field: "type", title: "Type" }, { field: "class", title: "Class" }]} data={dataList} height="70vh" />
        </>
      }
    />
  );
}

export function LogsScreen() {
  const [filter, setFilter] = useState("all");
  const [n, setN] = useState([300]);
  return (
    <div className="grid grid-cols-12 gap-x-[30px]">
      <div className="col-span-12 md:col-span-4">
        <div className="flex flex-col gap-[15px] border border-well-border bg-well p-[19px]">
          <h4 className="text-lg">
            <T k="logs_title" />
          </h4>
          <Field label="logs_to_show">
            <Slider min={1} max={1000} step={100} value={n} onValueChange={setN} />
          </Field>
          <RadioField
            label="logs_filter"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "error", label: "logs_filter_error" },
              { value: "warning", label: "logs_filter_warning" },
              { value: "message", label: "logs_filter_message" },
              { value: "log", label: "logs_filter_log" },
              { value: "all", label: "logs_filter_all" },
            ]}
          />
          <AmButton icon="archive" k="logs_download" className="self-start" />
        </div>
      </div>
      <div className="col-span-12 md:col-span-8">
        <DataTable columns={[{ field: "time" }, { field: "type" }, { field: "msg" }]} data={logs} height="70vh" />
      </div>
    </div>
  );
}

export function SettingsScreen() {
  const [adv, setAdv] = useState(false);
  const [admin, setAdmin] = useState(true);
  return (
    <div className="grid grid-cols-12">
      <div className="col-span-12 flex flex-col gap-[15px] border border-well-border bg-well p-[19px] md:col-span-4">
        <h4 className="text-lg">AccessMod 5.9.2-beta.6</h4>
        <CheckboxField label="settings_advanced_tools" checked={adv} onChange={setAdv} />
        <CheckboxField label="settings_admin_tools" checked={admin} onChange={setAdmin} />
        {admin && (
          <>
            <h3 className="text-2xl">
              <T k="settings_system_info_title" />
            </h3>
            <div className="flex flex-wrap gap-1">
              <AmButton k="settings_restart_am_btn" />
              <AmButton k="settings_clear_cache_restart_btn" />
              <AmButton k="settings_clear_archives_btn" />
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export function AboutScreen() {
  return (
    <div className="max-w-3xl space-y-2.5">
      <h4 className="text-lg">
        <T k="about_license" />
      </h4>
      <p>
        <T k="about_license_text" />
      </p>
      <h4 className="text-lg">
        <T k="about_acknowledge" />
      </h4>
      <p>
        <T k="about_acknowledge_text" />
      </p>
    </div>
  );
}
