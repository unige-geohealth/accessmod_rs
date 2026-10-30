// Analysis › Zonal statistics (AM5 module 5): population covered within
// travel-time limits, per zone. Update runs the analysis; its table is shown
// on the right, read page by page, with CSV and JSON downloads.
import { Columns } from "@accessmod/ui/am/columns";
import { DataTable } from "@accessmod/ui/am/data-table";
import { CheckboxField, TextField } from "@accessmod/ui/am/form";
import { LinkActions, MessageList } from "@accessmod/ui/am/messages";
import { CenterTitle, SectionCard } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { useMemo, useState } from "react";
import schema from "../../../schemas/jobs/zonal-statistics.json";
import { api } from "../../api/client";
import { useProgress, useRows, useRuns } from "../../api/queries";
import type { Api } from "../../generated/types";
import { datasetOptions, fieldClasses } from "../../state/catalogue";
import { draftStore, sessionStore } from "../../state/drafts";
import { runView } from "../../state/run";
import { useStore } from "../../state/store";
import { ANALYSIS, buildJob, EMPTY, updateZonal, type ZonalDraft, zonalOutput, zonalTable } from "../../state/zonal";
import { withZoneDefaults } from "../../state/zones";
import { DatasetSelect, fieldsOf, firstOf, Pager, useCatalogue, ZoneInputs } from "./inputs";
import { ValidationCard } from "./validation-card";

const POPULATION = fieldClasses(schema, "population");
const TRAVEL_TIME = fieldClasses(schema, "travel_time");
const ZONES = fieldClasses(schema, "zones");

/** Rows per page of the long table. */
const PAGE = 200;
/** Rows read at most for one column per time step (zones × limits): one server page. */
const WIDE = 1000;

/** AM5 hint under the limits: the range of the travel time, whole minutes. */
function rangeHint(dataset: Api.Dataset | undefined): string | null {
  const stats = dataset?.summary?.kind === "raster" ? dataset.summary.stats : undefined;
  if (stats?.min == null || stats.max == null) return null;
  return `min=${Math.floor(stats.min / 60)}, max=${Math.ceil(stats.max / 60)}`;
}

export function Zonal({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updateZonal);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId, dem } = useCatalogue(project);
  const options = {
    population: datasetOptions(t, datasets, POPULATION),
    travelTime: datasetOptions(t, datasets, TRAVEL_TIME),
    zones: datasetOptions(t, datasets, ZONES),
  };
  const fields = (id: number | null) => fieldsOf(byId, id);
  const draft: ZonalDraft = {
    ...stored,
    population: stored.population ?? firstOf(options.population),
    travelTime: stored.travelTime ?? firstOf(options.travelTime),
    zones: withZoneDefaults(stored.zones, firstOf(options.zones), fields),
  };
  const built = buildJob(draft, { datasets: byId, dem });
  const hint = rangeHint(draft.travelTime === null ? undefined : byId.get(draft.travelTime));
  const ready = draft.population !== null && draft.travelTime !== null && draft.zones.layer !== null;

  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <DatasetSelect label="analysis_select_pop_raster" options={options.population} value={draft.population} onChange={(id) => dispatch({ type: "population", id })} />
            <DatasetSelect
              label="analysis_select_travel_time_raster"
              options={options.travelTime}
              value={draft.travelTime}
              onChange={(id) => dispatch({ type: "travelTime", id })}
            />
            <ZoneInputs options={options.zones} input={draft.zones} fields={fields(draft.zones.layer)} onMessage={(message) => dispatch({ type: "zones", message })} />
            {ready && (
              <div className="flex flex-col gap-1">
                <TextField
                  label="analysis_select_max_travel_time"
                  help="analysis_select_max_travel_time_desc"
                  value={draft.minutes}
                  onChange={(text) => dispatch({ type: "minutes", text })}
                />
                {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
              </div>
            )}
          </SectionCard>

          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <CheckboxField
              label="analysis_zonal_stat_check_table_wide"
              help="analysis_zonal_stat_check_table_wide_desc"
              checked={draft.wide}
              onChange={(on) => dispatch({ type: "wide", on })}
            />
          </SectionCard>

          <ValidationCard
            project={project}
            analysis={ANALYSIS}
            issues={built.issues}
            notices={[]}
            document={built.document}
            tags={[]}
            datasets={byId}
            button="analysis_validation_update_btn"
            dialog={false}
          />
        </>
      }
      right={<ZonalResult project={project} ready={ready} wide={draft.wide} datasets={byId} />}
    />
  );
}

/** The table of the latest Update. */
function ZonalResult({ project, ready, wide, datasets }: { project: string; ready: boolean; wide: boolean; datasets: ReadonlyMap<number, Api.Dataset> }) {
  const { run } = useStore(sessionStore(project, ANALYSIS), (s) => s);
  const view = runView(useRuns(project).data, useProgress(project).data, run);
  const output = zonalOutput(view.state === "done" ? view.record : undefined, datasets);
  return (
    <>
      <CenterTitle title="analysis_zonal_stat_title" sub="analysis_zonal_stat_sub" />
      <div className="flex flex-col gap-2 px-[15px]">
        {!ready ? (
          <MessageList messages={[{ kind: "info", k: "analysis_zonal_stat_add" }]} />
        ) : output === null ? (
          <MessageList messages={[{ kind: "info", k: "analysis.zonal_update" }]} />
        ) : (
          // Keyed by output: a new table starts at its first page.
          <ZonalTable key={output} project={project} output={output} wide={wide} />
        )}
      </div>
    </>
  );
}

/** An output table read through `rows`: pages of the long table, or the whole table pivoted when it is small enough. */
function ZonalTable({ project, output, wide }: { project: string; output: number; wide: boolean }) {
  const [offset, setOffset] = useState(0);
  // One column per time step needs the whole table in one read; else pages of the long table.
  const whole = useRows(project, wide ? output : null, { limit: WIDE }).data;
  const pivot = whole !== undefined && whole.rows.length >= whole.total;
  const page = useRows(project, wide && (whole === undefined || pivot) ? null : output, { offset, limit: PAGE }).data;
  const rows = pivot ? whole : page;
  const table = useMemo(() => zonalTable(rows, pivot), [rows, pivot]);
  return (
    <>
      {wide && whole !== undefined && !pivot && <MessageList messages={[{ kind: "info", k: "analysis.wide_too_large" }]} />}
      <LinkActions
        actions={(["csv", "json"] as const).map((format) => ({
          icon: "download" as const,
          k: format === "csv" ? ("analysis.download_csv" as const) : ("analysis.download_json" as const),
          onClick: () => {
            window.location.href = api.downloadUrl(project, output, format);
          },
        }))}
      />
      <DataTable toolbar={false} layout="fitData" columns={table.columns.map((field) => ({ field }))} data={table.data} height="70vh" />
      {!pivot && <Pager offset={offset} page={PAGE} total={rows?.total ?? 0} onOffset={setOffset} />}
    </>
  );
}
