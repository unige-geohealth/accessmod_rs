// Analysis (AM5 `amAnalysisAccessibility`): one tab per analysis, data
// input, settings and validation on the left, the tables of the analysis on
// the right. The draft of each tab is a store per project (state/), so it
// survives navigation; datasets, rows, validation and runs are server state.
import { Columns } from "@accessmod/ui/am/columns";
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { CheckboxField, RadioField, SelectField, TextField } from "@accessmod/ui/am/form";
import { MessageDialog } from "@accessmod/ui/am/message-dialog";
import { AmButton, LinkActions, type Message, MessageList } from "@accessmod/ui/am/messages";
import { CenterTitle, SectionCard, SubTitle } from "@accessmod/ui/am/section";
import { type SectionTab, SectionTabs } from "@accessmod/ui/am/section-tabs";
import { T, useI18n } from "@accessmod/ui/i18n";
import { Progress } from "@accessmod/ui/ui/progress";
import { useMemo, useState } from "react";
import travelTimeSchema from "../../schemas/jobs/travel-time.json";
import { api, issuesOf } from "../api/client";
import { keys, queryClient, useDatasets, useProgress, useProject, useRows, useRuns, useValidation } from "../api/queries";
import { navigate, type Route } from "../app/route";
import type { Api } from "../generated/types";
import {
  type AccessibilityDraft,
  type AccessibilityMessage,
  ANALYSIS,
  buildJob,
  EMPTY,
  scenarioRows,
  updateAccessibility,
  withDefaults,
} from "../state/accessibility";
import { classLabel, datasetOptions, fieldClasses, pairedTable } from "../state/catalogue";
import { checkedOnPage, checkPage, count, type FacilitySelection } from "../state/facility-selection";
import { busy, minutes, percent, runView, type RunView } from "../state/run";
import {
  editRow,
  hasZero,
  initialRows,
  labelsOf,
  landCoverClasses,
  MODES,
  mergeTable,
  type ScenarioRow,
  tableNotices,
  tableRows,
} from "../state/scenario";
import { createStore, type Store, useStore } from "../state/store";
import { useFmt } from "./common";
import { NotYet } from "./other";

type Tab = "accessibility" | "coverage" | "referral" | "zonal" | "scalingUp" | "optimization";

const TABS: SectionTab<Tab>[] = [
  { value: "accessibility", label: "analysis_accessibility_main_tab", icon: "accessibility" },
  { value: "coverage", label: "analysis_coverage_main_tab", icon: "coverage" },
  { value: "referral", label: "analysis_referral_main_tab", icon: "referral" },
  { value: "zonal", label: "analysis_zonal_stats_main_tab", icon: "zonal" },
  { value: "scalingUp", label: "analysis_scalingup_main_tab", icon: "scalingUp" },
  { value: "optimization", label: "analysis_optimization_main_tab", icon: "optimization" },
];

const isTab = (v: string | null): v is Tab => TABS.some((t) => t.value === v);

export function AnalysisScreen({ route, project }: { route: Route; project: string }) {
  const tab = isTab(route.panel) ? route.panel : "accessibility";
  return (
    <div className="flex flex-col gap-5">
      <SectionTabs tabs={TABS} value={tab} onChange={(v) => navigate({ ...route, panel: v, query: {} })} />
      {tab === "accessibility" ? <Accessibility project={project} /> : <NotYet />}
    </div>
  );
}

/** Drafts and run sessions per project, kept while the client runs. */
const drafts = new Map<string, Store<AccessibilityDraft, AccessibilityMessage>>();

function draftStore(project: string) {
  let store = drafts.get(project);
  if (store === undefined) {
    store = createStore(EMPTY, updateAccessibility);
    drafts.set(project, store);
  }
  return store;
}

/** The run a screen started, and whether its outcome was acknowledged. */
interface Session {
  run: number | null;
  acknowledged: boolean;
}
type SessionMessage = { type: "started"; run: number } | { type: "acknowledged" };

const updateSession = (session: Session, message: SessionMessage): Session =>
  message.type === "started" ? { run: message.run, acknowledged: false } : { ...session, acknowledged: true };

const sessions = new Map<string, Store<Session, SessionMessage>>();

function sessionStore(key: string) {
  let store = sessions.get(key);
  if (store === undefined) {
    store = createStore<Session, SessionMessage>({ run: null, acknowledged: true }, updateSession);
    sessions.set(key, store);
  }
  return store;
}

/** Rows of a table read whole (land cover labels, scenario tables): small by nature. */
const WHOLE = { limit: 1000 };

const LAND_COVER = fieldClasses(travelTimeSchema, "land_cover");
const FACILITIES = fieldClasses(travelTimeSchema, "facilities");

function Accessibility({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const datasets = useDatasets(project).data;
  const info = useProject(project).data;
  const byId = useMemo(() => new Map((datasets ?? []).map((d) => [d.id, d])), [datasets]);

  const landCoverOptions = datasetOptions(t, datasets, LAND_COVER);
  const facilityOptions = datasetOptions(t, datasets, FACILITIES);
  const scenarioOptions = datasetOptions(t, datasets, ["tScenario"]);
  const first = (options: { value: string }[]) => (options[0] ? Number(options[0].value) : null);
  const draft = withDefaults(stored, {
    landCover: first(landCoverOptions),
    facilities: first(facilityOptions),
    fieldsOf: (id) => {
      const summary = id === null ? undefined : byId.get(id)?.summary;
      return summary?.kind === "vector" ? summary.fields : [];
    },
  });
  const scenarioTable = stored.scenarioTable ?? first(scenarioOptions);

  const landCover = draft.landCover === null ? undefined : byId.get(draft.landCover);
  const labels = useRows(project, pairedTable(datasets, landCover, "tLandCover")?.id ?? null, WHOLE);
  const initial = useMemo(() => initialRows(landCoverClasses(landCover), labelsOf(labels.data)), [landCover, labels.data]);
  const scenario = useMemo(() => scenarioRows(draft, initial), [draft, initial]);
  const built = useMemo(() => buildJob(draft, { datasets: byId, dem: info?.dem ?? null, scenario }), [draft, byId, info?.dem, scenario]);
  const facilityFields = draft.facilities === null ? [] : (() => {
    const summary = byId.get(draft.facilities)?.summary;
    return summary?.kind === "vector" ? summary.fields : [];
  })();

  // A choice equal to what is shown changes nothing (defaults included).
  const choose = (field: "landCover" | "facilities" | "scenarioTable", value: string) => {
    const id = value === "" ? null : Number(value);
    if (field === "scenarioTable") return dispatch({ type: "scenarioTable", id });
    if (id !== draft[field]) dispatch({ type: field, id } as AccessibilityMessage);
  };

  return (
    <Columns
      left={
        <>
          <SectionCard title="analysis_data_input_title" sub="analysis_data_input_sub" icon="dataInput">
            <SelectField
              label="analysis_select_merged_lc_raster"
              options={landCoverOptions}
              value={draft.landCover === null ? undefined : String(draft.landCover)}
              onChange={(v) => choose("landCover", v)}
            />
            <SelectField
              label="analysis_select_scenario_table"
              options={scenarioOptions}
              value={scenarioTable === null ? undefined : String(scenarioTable)}
              onChange={(v) => choose("scenarioTable", v)}
            />
            <SelectField
              label="analysis_select_health_facility_vector"
              options={facilityOptions}
              value={draft.facilities === null ? undefined : String(draft.facilities)}
              onChange={(v) => choose("facilities", v)}
            />
            {facilityFields.length > 0 && (
              <SelectField
                indent
                label="analysis_select_health_facility_id_field"
                options={facilityFields.map((f) => ({ value: f, label: f }))}
                value={draft.idField}
                onChange={(field) => {
                  if (field !== draft.idField) dispatch({ type: "idField", field });
                }}
              />
            )}
          </SectionCard>

          <SectionCard title="analysis_settings_title" sub="analysis_settings_sub" icon="analysisSettings">
            <RadioField
              label="analysis_settings_type"
              value={draft.method}
              onChange={(v) => dispatch({ type: "method", method: v as AccessibilityDraft["method"] })}
              options={[
                { value: "isotropic", label: "analysis_settings_isotropic" },
                { value: "anisotropic", label: "analysis_settings_anisotropic" },
              ]}
            />
            <RadioField
              label="analysis_settings_travel_direction"
              value={draft.direction}
              onChange={(v) => dispatch({ type: "direction", direction: v as AccessibilityDraft["direction"] })}
              options={[
                { value: "from_facilities", label: "analysis_settings_travel_from_hf" },
                { value: "towards_facilities", label: "analysis_settings_travel_to_hf" },
              ]}
            />
            <CheckboxField
              label="analysis_settings_knight_move"
              help="analysis_settings_knight_move_desc"
              checked={draft.knight}
              onChange={(on) => dispatch({ type: "knight", on })}
            />
            <TextField label="analysis_settings_max_travel_time" type="number" value={draft.minutes} onChange={(text) => dispatch({ type: "minutes", text })} />
            <TextField label="analysis_settings_add_tag" value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
          </SectionCard>

          <ValidationCard
            project={project}
            analysis={ANALYSIS}
            issues={built.issues}
            notices={hasZero(scenario) ? ["srv_analysis_accessibility_tbl_speed_has_zero"] : []} document={built.document} tags={built.tags} datasets={byId} />
        </>
      }
      right={
        <>
          <ScenarioSection project={project} scenario={scenario} scenarioTable={scenarioTable} dispatch={dispatch} hasLandCover={landCover !== undefined} />
          <FacilitySection
            project={project}
            facilities={draft.facilities}
            landCover={draft.landCover}
            idField={draft.idField}
            selection={draft.selection}
            scenario={scenario}
            onSelection={(selection) => dispatch({ type: "selection", selection })}
          />
        </>
      }
    />
  );
}

const SCENARIO_COLUMNS: Column[] = [
  { field: "class" },
  { field: "label" },
  { field: "speed", editable: "number" },
  { field: "mode", editable: { values: [...MODES] } },
];

function ScenarioSection({
  project,
  scenario,
  scenarioTable,
  dispatch,
  hasLandCover,
}: {
  project: string;
  scenario: readonly ScenarioRow[];
  scenarioTable: number | null;
  dispatch: (message: AccessibilityMessage) => void;
  hasLandCover: boolean;
}) {
  const f = useFmt();
  const table = useRows(project, scenarioTable, WHOLE);
  const notices = tableNotices(scenario, table.data);
  const messages: Message[] = [
    ...(notices.unknownMode ? [{ kind: "info" as const, text: f("srv_analysis_access_unmatched_transport_warning", MODES.join(","), "WALKING") }] : []),
    ...(notices.unmatched ? [{ kind: "info" as const, k: "srv_analysis_access_unmatched_scenario_labels_warning" as const }] : []),
  ];
  const selected = useMemo(() => tableRows(table.data), [table.data]);
  return (
    <>
      <CenterTitle title="analysis_travel_scenario_title" sub="analysis_travel_scenario_sub" />
      <div className="px-[15px]">
        <SubTitle k="analysis_travel_scenario_processed" />
        {hasLandCover ? (
          <>
            <LinkActions
              actions={[
                { icon: "reset", k: "analysis_travel_scenario_reset", onClick: () => dispatch({ type: "resetScenario" }) },
                {
                  icon: "edit",
                  k: "analysis_travel_scenario_import_selected_table",
                  onClick: () => dispatch({ type: "scenario", rows: mergeTable(scenario, selected) }),
                },
              ]}
            />
            <DataTable
              columns={SCENARIO_COLUMNS}
              data={scenario as unknown as Row[]}
              height="300px"
              rowId="class"
              onCellEdited={(row, field, value) => {
                if (field === "speed" || field === "mode") dispatch({ type: "scenario", rows: editRow(scenario, Number(row.class), field, value) });
              }}
            />
          </>
        ) : (
          <MessageList messages={[{ kind: "info", k: "analysis_travel_scenario_add_lc" }]} />
        )}
        <SubTitle k="analysis_travel_scenario_selected_table" />
        {scenarioTable === null ? (
          <MessageList messages={[{ kind: "info", k: "analysis_travel_scenario_add_table" }]} />
        ) : (
          <DataTable columns={SCENARIO_COLUMNS.map((c) => ({ field: c.field }))} data={selected as unknown as Row[]} height="300px" />
        )}
        {messages.length > 0 && <MessageList messages={messages} />}
      </div>
    </>
  );
}

/** Facilities shown per page of the selection table. */
const PAGE = 200;

/** Every identifier matching the filter, page after page (bounded pages). */
async function everyId(project: string, dataset: number, where: string, idField: string): Promise<unknown[]> {
  const ids: unknown[] = [];
  for (let offset = 0; ; offset += 1000) {
    const page = await api.rows(project, dataset, { offset, limit: 1000, where });
    const at = page.columns.indexOf(idField);
    ids.push(...page.rows.map((r) => r[at]));
    if (page.rows.length === 0 || offset + page.rows.length >= page.total) return ids;
  }
}

function FacilitySection({
  project,
  facilities,
  landCover,
  idField,
  selection,
  scenario,
  onSelection,
}: {
  project: string;
  facilities: number | null;
  landCover: number | null;
  idField: string;
  selection: FacilitySelection;
  scenario: readonly ScenarioRow[];
  onSelection: (selection: FacilitySelection) => void;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const [offset, setOffset] = useState(0);
  const [filter, setFilter] = useState(selection.where);
  const [failure, setFailure] = useState<string[]>([]);
  const page = useRows(project, facilities, {
    offset,
    limit: PAGE,
    where: selection.where,
    sample: landCover === null ? [] : [landCover],
  });
  const data = page.data;
  const at = data?.columns.indexOf(idField) ?? -1;
  const ids = useMemo(() => (data && at >= 0 ? data.rows.map((r) => r[at]) : []), [data, at]);
  const speeds = useMemo(() => new Map(scenario.map((r) => [r.class, r.speed])), [scenario]);
  const rows = useMemo(() => {
    if (!data) return [];
    return data.rows.map((values, i) => {
      const cls = data.samples?.[i]?.[0] ?? null;
      const row: Row = {
        amOnBarrier: cls === null ? "yes" : "no",
        amOnZero: cls !== null && (speeds.get(cls) ?? 0) === 0 ? "yes" : "no",
        amCatLandCover: cls,
      };
      data.columns.forEach((c, k) => {
        const v = values[k];
        row[c] = v === null || v === undefined ? null : typeof v === "object" ? JSON.stringify(v) : (v as string | number | boolean);
      });
      return row;
    });
  }, [data, speeds]);
  const columns: Column[] = useMemo(() => {
    const rest = (data?.columns ?? []).filter((c) => c !== idField).map((field) => ({ field }));
    return [{ field: idField }, { field: "amOnBarrier" }, { field: "amOnZero" }, { field: "amCatLandCover" }, ...rest];
  }, [data?.columns, idField]);

  if (facilities === null) {
    return (
      <>
        <CenterTitle title="analysis_facility_selection_title" sub="analysis_facility_selection_sub" />
        <MessageList messages={[{ kind: "info", k: "analysis_facility_selection_add_data" }]} />
      </>
    );
  }

  const total = data?.total ?? 0;
  const link = "px-1 text-link hover:text-link-hover hover:underline disabled:opacity-50 disabled:no-underline";
  const changePage = async (checked: Row[]) => {
    setFailure([]);
    try {
      const all = selection.mode === "all" ? await everyId(project, facilities, selection.where, idField) : undefined;
      onSelection(checkPage(selection, ids, checked.map((r) => r[idField]), all));
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };

  return (
    <>
      <CenterTitle title="analysis_facility_selection_title" sub="analysis_facility_selection_sub" />
      <div className="flex flex-col gap-2 px-[15px]">
        <SubTitle k="analysis_facility_selected_facilities" />
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            setOffset(0);
            onSelection({ mode: "all", where: filter });
          }}
        >
          <div className="min-w-60 flex-1">
            <TextField label="analysis.facility_filter" value={filter} onChange={setFilter} />
          </div>
          <AmButton type="submit" k="analysis.apply_filter" />
        </form>
        {page.isError && <MessageList messages={issuesOf(page.error, t).map((text) => ({ kind: "issue", text }))} />}
        {failure.length > 0 && <MessageList messages={failure.map((text) => ({ kind: "issue", text }))} />}
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <button type="button" className={link} onClick={() => onSelection({ mode: "all", where: selection.where })}>
            [{t("table.all")}]
          </button>
          <button type="button" className={link} onClick={() => onSelection({ mode: "ids", where: selection.where, ids: [] })}>
            [{t("table.none")}]
          </button>
          <span>{f("analysis.facilities_selected", count(selection, total), total)}</span>
        </div>
        {data && total === 0 ? (
          <MessageList messages={[{ kind: "info", k: "analysis.no_rows" }]} />
        ) : (
          <DataTable
            selectable
            toolbar={false}
            rowId={idField}
            selected={checkedOnPage(selection, ids)}
            onSelectionChange={(checked) => void changePage(checked)}
            columns={columns}
            data={rows}
            height="420px"
          />
        )}
        {total > PAGE && (
          <div className="flex items-center gap-2 text-sm">
            <button type="button" className={link} disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE))}>
              [{t("analysis.previous")}]
            </button>
            <span>{f("analysis.rows_page", offset + 1, Math.min(offset + PAGE, total), total)}</span>
            <button type="button" className={link} disabled={offset + PAGE >= total} onClick={() => setOffset(offset + PAGE)}>
              [{t("analysis.next")}]
            </button>
          </div>
        )}
      </div>
    </>
  );
}

/** AM5 "Validation": what is missing, the server's checks, then Compute, progress and the outcome. */
function ValidationCard({
  project,
  analysis,
  issues,
  notices,
  document,
  tags,
  datasets,
}: {
  project: string;
  analysis: string;
  issues: Parameters<typeof T>[0]["k"][];
  /** AM5 information messages (not blocking). */
  notices: Parameters<typeof T>[0]["k"][];
  /** Complete job document; `null` while incomplete. */
  document: Record<string, unknown> | null;
  tags: string[];
  datasets: ReadonlyMap<number, Api.Dataset>;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const validation = useValidation(project, analysis, document);
  const session = sessionStore(`${project}/${analysis}`);
  const { run, acknowledged } = useStore(session, (s) => s);
  const runs = useRuns(project).data;
  const progress = useProgress(project).data;
  const view: RunView = runView(runs, progress, run);
  const [failure, setFailure] = useState<string[]>([]);

  const messages: Message[] = [
    ...issues.map((k) => ({ kind: "issue" as const, k })),
    ...notices.map((k) => ({ kind: "info" as const, k })),
    ...(validation.isError ? issuesOf(validation.error, t).map((text) => ({ kind: "issue" as const, text })) : []),
    ...((validation.data?.warnings as { message: string }[] | undefined) ?? []).map((w) => ({ kind: "info" as const, text: w.message })),
    ...failure.map((text) => ({ kind: "issue" as const, text })),
  ];
  if (view.state === "failed") {
    const key = view.record.status === "cancelled" ? "analysis.run_cancelled" : "analysis.run_failed";
    messages.push({ kind: "issue", k: key }, ...(view.record.error?.issues ?? []).map((text) => ({ kind: "issue" as const, text })));
  }
  const ready = document !== null && validation.isSuccess && !busy(view);

  const compute = async () => {
    setFailure([]);
    try {
      if (document === null) return;
      const record = await api.command("run.start", { project, analysis, document, tags });
      await queryClient.invalidateQueries({ queryKey: keys.runs(project) });
      session.dispatch({ type: "started", run: record.id });
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };
  const cancel = async () => {
    if (run === null) return;
    try {
      await api.command("run.cancel", { project, id: run });
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };
  const download = () => {
    const blob = new Blob([`${JSON.stringify(document, null, 2)}\n`], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = window.document.createElement("a");
    a.href = url;
    a.download = `${analysis}.json`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <SectionCard title="analysis_validation_title" sub="analysis_validation_sub" icon="validation">
      {messages.length > 0 ? (
        <MessageList messages={messages} />
      ) : (
        <p className="text-sm">
          <T k={validation.isFetching ? "analysis.validating" : "analysis.valid"} />
        </p>
      )}
      {busy(view) ? (
        <RunProgress view={view} onCancel={() => void cancel()} />
      ) : (
        <AmButton variant="danger" block disabled={!ready} k="analysis_validation_compute_btn" onClick={() => void compute()} />
      )}
      {document !== null && <LinkActions actions={[{ icon: "download", k: "analysis.download_job", onClick: download }]} />}
      <MessageDialog
        open={view.state === "done" && !acknowledged}
        onOpenChange={(open) => {
          if (!open) session.dispatch({ type: "acknowledged" });
        }}
        title="srv_analysis_accessibility_process_finished_message"
      >
        {view.state === "done" && (
          <>
            <p>{f("srv_analysis_accessibility_process_finished_timing", minutes(view.record).toFixed(3))}</p>
            <ul className="list-disc pl-10">
              {view.record.datasets.map((id) => {
                const d = datasets.get(id);
                return <li key={id}>{d ? `${classLabel(t, d.class)} [${d.tags.join(" ")}]` : `#${id}`}</li>;
              })}
            </ul>
          </>
        )}
      </MessageDialog>
    </SectionCard>
  );
}

function RunProgress({ view, onCancel }: { view: RunView; onCancel: () => void }) {
  const f = useFmt();
  const stage = view.state === "running" ? view.progress : undefined;
  const value = percent(stage);
  return (
    <div className="flex flex-col gap-1.5" role="status">
      <Progress value={value ?? 100} className={value === undefined ? "animate-pulse" : undefined} />
      <span className="text-xs text-muted-foreground">{stage ? f("analysis.running", stage.stage) : f("analysis.queued")}</span>
      <AmButton block k="analysis.cancel" onClick={onCancel} />
    </div>
  );
}
