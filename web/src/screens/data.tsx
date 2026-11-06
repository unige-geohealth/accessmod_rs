// Data (AM5 `amManageData`): import, filter, rename, archive and delete on
// the left; "Available data" on the right. The table's selection and tag
// edits are the screen's document (state/data-document.ts); filters are in
// the URL; datasets come from the server.
import { Columns } from "@accessmod/ui/am/columns";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { DataTable, type Row } from "@accessmod/ui/am/data-table";
import { CheckboxField, RadioField, SelectField, TextField } from "@accessmod/ui/am/form";
import { AmButton, FilePicker, InfoList, LinkActions, MessageList } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { CenterTitle } from "@accessmod/ui/am/section";
import { T, useI18n } from "@accessmod/ui/i18n";
import { type Dispatch, useMemo, useReducer, useState } from "react";
import { api, issuesOf } from "../api/client";
import { runResult } from "../api/tasks";
import { keys, queryClient, useArchives, useDatasets, useRuns } from "../api/queries";
import { navigate, type Route } from "../app/route";
import type { Api } from "../generated/types";
import { useBatch } from "../state/batch";
import {
  ACCEPT,
  CLASSES,
  classLabel,
  type DataFilter,
  type DataRow,
  filterFromQuery,
  filterToQuery,
  jobRow,
  kindOf,
  matches,
  normalizeTags,
  type Role,
  runRoles,
  sortRows,
  toRow,
} from "../state/catalogue";
import {
  type DataDocument,
  type DataMessage,
  EMPTY,
  incompleteRuns,
  selectedDatasets,
  selectedKeys,
  selectedRuns,
  tagChanges,
  updateData,
} from "../state/data-document";
import { analysisLabel } from "../state/jobs";
import { doneRuns, lastRun } from "../state/run";
import { busy, useTransfer } from "../state/transfer";
import { BatchStatus, downloadJson, TransferStatus, useFmt } from "./common";

const ROLE_KEYS = { input: "data.role_input", output: "data.role_output", job: "data.kind_job" } as const satisfies Record<Role, string>;

export function DataScreen({ route, project }: { route: Route; project: string }) {
  const { t } = useI18n();
  const datasets = useDatasets(project);
  const runs = useRuns(project).data;
  const finished = useMemo(() => doneRuns(runs), [runs]);
  const [doc, dispatch] = useReducer(updateData, EMPTY);
  const filter = filterFromQuery(route.query);
  const filterRun = filter.run === null ? undefined : finished.find((r) => r.id === filter.run);
  const roles = useMemo(() => runRoles(filterRun), [filterRun]);
  const renaming = route.panel === "rename";
  const label = useMemo(() => (dataset: Api.Dataset) => `${classLabel(t, dataset.class)} [${dataset.tags.join(" ")}]`, [t]);

  const list = datasets.data;
  const rows = useMemo(() => {
    const all = sortRows([...(list ?? []).map((d) => toRow(d, t)), ...finished.map((r) => jobRow(r, t))]);
    return all
      .filter((r) => matches(r, filter, roles))
      .map((r) => {
        const role = roles.get(r.key);
        return { ...r, tags: r.kind === "job" ? r.tags : (doc.edits[r.id] ?? r.tags), role: role ? t(ROLE_KEYS[role]) : "" };
      });
    // the filter is a value of the URL query
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list, finished, t, doc.edits, filter.type, filter.text, filter.tags, filter.run, roles]);
  // Stable between renders: the table resyncs its selection when it changes.
  const selected = useMemo(
    () =>
      selectedKeys(
        doc,
        rows.map((r) => r.key),
        filter.run,
      ),
    [doc, rows, filter.run],
  );

  const setFilter = (next: DataFilter) => navigate({ ...route, query: filterToQuery(next) });

  return (
    <Columns
      wide
      left={
        <PanelAccordion
          value={route.panel ?? ""}
          onValueChange={(v) => navigate({ ...route, panel: v === "" ? null : v })}
          items={[
            { value: "import", icon: "add", title: "data_import_data", content: <ImportPanel project={project} /> },
            { value: "filter", icon: "filter", title: "data_filter_data", content: <FilterPanel project={project} filter={filter} runs={finished} onChange={setFilter} /> },
            {
              value: "rename",
              icon: "rename",
              title: "data_rename_data",
              content: <RenamePanel project={project} doc={doc} selected={selected} dispatch={dispatch} datasets={list ?? []} label={label} />,
            },
            {
              value: "archive",
              icon: "archive",
              title: "data_archive_data",
              content: <ArchivePanel project={project} selected={selected} datasets={list ?? []} runs={finished} label={label} />,
            },
            {
              value: "delete",
              icon: "delete",
              title: "data_delete_data",
              content: <DeletePanel project={project} doc={doc} selected={selected} dispatch={dispatch} datasets={list ?? []} runs={finished} label={label} />,
            },
          ]}
        />
      }
      right={
        <>
          <CenterTitle title="data_available_title" sub="data_available_sub" />
          {datasets.isError && <MessageList messages={issuesOf(datasets.error, t).map((text) => ({ kind: "issue", text }))} />}
          <DataTable
            selectable
            rowId="key"
            selected={selected as string[]}
            onSelectionChange={(chosen: Row[]) => dispatch({ type: "select", run: filter.run, keys: chosen.map((r) => String(r.key)) })}
            onCellEdited={(row, field, value) => {
              if (field === "tags" && row.kind !== "job") dispatch({ type: "editTags", id: Number(row.id), text: String(value ?? "") });
            }}
            columns={[
              { field: "tags", title: t("data.column_tags"), editable: renaming, editableRow: isDatasetRow },
              { field: "type", title: t("data.column_type") },
              { field: "class", title: t("data.column_class") },
              ...(filter.run === null ? [] : [{ field: "role", title: t("data.column_role") }]),
            ]}
            data={rows as DataRow[]}
            height="70vh"
          />
        </>
      }
    />
  );
}

function ImportPanel({ project }: { project: string }) {
  const { t } = useI18n();
  const f = useFmt();
  const [cls, setCls] = useState<Api.DataClass | undefined>(undefined);
  const [tagText, setTagText] = useState("");
  const { state, start } = useTransfer();
  const tags = normalizeTags(tagText);
  const ready = cls !== undefined && tags.length > 0 && !busy(state);
  const options = CLASSES.filter((c) => c.importable).map((c) => ({ value: c.class, label: classLabel(t, c.class) }));

  const importFiles = (files: File[]) => {
    if (cls === undefined) return;
    void start(files, async (upload, queued) => {
      const run = await api.command("dataset.import", { project, source: { upload: { id: upload.id } }, class: cls, tags });
      queued({ kind: "run", project, id: run.id });
      const record = await runResult<Api.Dataset>(project, run.id);
      await queryClient.invalidateQueries({ queryKey: keys.datasets(project) });
      return `${classLabel(t, record.class)} [${record.tags.join(" ")}]`;
    });
  };

  return (
    <div className="flex flex-col gap-[15px]">
      <SelectField label="data_select_data_class" options={options} value={cls} onChange={(v) => setCls(v as Api.DataClass)} />
      <TextField label="data_add_tag" value={tagText} onChange={setTagText} />
      {tags.length > 0 ? (
        <p className="text-xs text-muted-foreground">{f("data.tags_preview", tags.join(" "))}</p>
      ) : (
        <p className="text-xs text-muted-foreground">
          <T k="srv_data_short_missing_tag_notice" />
        </p>
      )}
      {cls === "rDem" && <InfoList items={["srv_data_import_new_dem_warning"]} />}
      <FilePicker
        block
        k="data_choose_import_btn"
        accept={cls === undefined ? undefined : ACCEPT[kindOf(cls)]}
        multiple={cls !== undefined && (kindOf(cls) === "vector" || kindOf(cls) === "raster")}
        disabled={!ready}
        onFiles={importFiles}
      />
      <TransferStatus transfer={state} doneKey="data.imported" />
    </div>
  );
}

/** Job rows have no tags of their own to edit: they are the run's. */
const isDatasetRow = (row: Row) => row.kind !== "job";

function FilterPanel({
  project,
  filter,
  runs,
  onChange,
}: {
  project: string;
  filter: DataFilter;
  /** Finished runs, newest first. */
  runs: readonly Api.RunRecord[];
  onChange: (f: DataFilter) => void;
}) {
  const { t } = useI18n();
  const last = lastRun(runs);
  const run = runs.find((r) => r.id === filter.run);
  const options = [
    { value: "", label: t("table.none") },
    ...runs.map((r) => ({ value: String(r.id), label: `#${r.id} ${analysisLabel(t, r.analysis)} [${r.tags.join(" ")}]` })),
  ];
  return (
    <div className="flex flex-col gap-[15px]">
      <RadioField
        inline
        label="data_type_data_format"
        value={filter.type}
        onChange={(type) => onChange({ ...filter, type: type as DataFilter["type"] })}
        options={[
          { value: "vector", label: "data_vector" },
          { value: "raster", label: "data_raster" },
          { value: "table", label: "data_table" },
          { value: "job", label: "data.kind_job" },
          { value: "all", label: "data_all" },
        ]}
      />
      <TextField label="data_filter_text" value={filter.text} onChange={(text) => onChange({ ...filter, text })} />
      <TextField label="data_filter_tag" value={filter.tags} onChange={(tags) => onChange({ ...filter, tags })} />
      {last !== null && (
        <CheckboxField label="data.last_results" checked={filter.run === last} onChange={(on) => onChange({ ...filter, run: on ? last : null })} />
      )}
      {runs.length > 0 && (
        <SelectField
          label="data.run"
          options={options}
          value={filter.run === null ? "" : String(filter.run)}
          onChange={(v) => onChange({ ...filter, run: v === "" ? null : Number(v) })}
        />
      )}
      {run && (
        <LinkActions
          actions={[
            {
              icon: "logs",
              k: "data.open_job",
              onClick: () => navigate({ project, section: "jobs", panel: null, query: { scope: "project", job: `${project}/${run.id}` } }),
            },
            { icon: "download", k: "analysis.download_job", onClick: () => downloadJson(`${run.analysis}-${project}-${run.id}`, run.job) },
          ]}
        />
      )}
    </div>
  );
}

function ArchivePanel({
  project,
  selected,
  datasets,
  runs,
  label,
}: {
  project: string;
  selected: readonly string[];
  datasets: readonly Api.Dataset[];
  runs: readonly Api.RunRecord[];
  label: (dataset: Api.Dataset) => string;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const archives = useArchives(project).data ?? [];
  const [prefix, setPrefix] = useState("");
  const [chosen, setChosen] = useState<string | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const [messages, setMessages] = useState<{ kind: "issue" | "info"; text: string }[]>([]);
  const chosenDatasets = selectedDatasets(selected, datasets);
  const chosenRuns = selectedRuns(selected).filter((id) => runs.some((r) => r.id === id));
  const incomplete = incompleteRuns(selected, runs);
  // The archive shown: the one picked if it still exists, else the newest.
  const current = archives.find((a) => a.name === chosen) ?? archives[0];
  const act = async (action: () => Promise<string | null>) => {
    setBusy(true);
    setMessages([]);
    try {
      const done = await action();
      if (done !== null) setMessages([{ kind: "info", text: done }]);
    } catch (error) {
      setMessages(issuesOf(error, t).map((text) => ({ kind: "issue" as const, text })));
    } finally {
      setBusy(false);
      await queryClient.invalidateQueries({ queryKey: keys.archives(project) });
    }
  };
  const creation = useTransfer();
  const create = () =>
    creation.run(async (queued) => {
      setMessages([]);
      const run = await api.command("archive.create", {
        project,
        name: prefix.trim(),
        datasets: chosenDatasets.map((d) => d.id),
        runs: chosenRuns,
      });
      queued({ kind: "run", project, id: run.id });
      const archive = await runResult<Api.Archive>(project, run.id);
      await queryClient.invalidateQueries({ queryKey: keys.archives(project) });
      setChosen(archive.name);
      return archive.name;
    });
  const creating = creation.state.state === "processing";
  const remove = (name: string) =>
    act(async () => {
      await api.command("archive.remove", { project, name });
      setChosen(undefined);
      return null;
    });
  return (
    <div className="flex flex-col gap-[15px]">
      <TextField label="data.archive_prefix" value={prefix} onChange={setPrefix} />
      {chosenDatasets.length + chosenRuns.length === 0 ? (
        <p className="text-muted-foreground">
          <T k="data_archive_data_selected_table" />
        </p>
      ) : (
        <ul className="list-inside list-disc">
          {chosenDatasets.map((d) => (
            <li key={d.id}>{label(d)}</li>
          ))}
          {runs
            .filter((r) => chosenRuns.includes(r.id))
            .map((r) => (
              <li key={`j${r.id}`}>{`${t("data.kind_job")} #${r.id} ${analysisLabel(t, r.analysis)} [${r.tags.join(" ")}]`}</li>
            ))}
        </ul>
      )}
      {incomplete.length > 0 && (
        <MessageList messages={[{ kind: "info", text: f("data.archive_incomplete", incomplete.map((r) => `#${r.id}`).join(", ")) }]} />
      )}
      <AmButton block k="data_archive_create" disabled={busy || creating || chosenDatasets.length + chosenRuns.length === 0} onClick={() => void create()} />
      <TransferStatus transfer={creation.state} doneKey="data.archive_created" />
      {messages.length > 0 && <MessageList messages={messages} />}
      <hr className="border-border" />
      {current === undefined ? (
        <p className="text-muted-foreground">
          <T k="data.archive_none" />
        </p>
      ) : (
        <>
          <SelectField
            label="data_archive_select"
            options={archives.map((a) => ({ value: a.name, label: `${a.name} (${(a.bytes / 1e6).toFixed(1)} MB)` }))}
            value={current.name}
            onChange={setChosen}
          />
          <p className="text-xs text-muted-foreground">
            <T k="data_download_delete_archive" />
          </p>
          <AmButton block k="data_archive_export" disabled={busy} onClick={() => (window.location.href = api.archiveUrl(project, current.name))} />
          <AmButton block variant="danger" k="data_archive_delete_btn" disabled={busy} onClick={() => void remove(current.name)} />
        </>
      )}
    </div>
  );
}

interface DocumentProps {
  project: string;
  doc: DataDocument;
  /** Keys of the selected rows. */
  selected: readonly string[];
  dispatch: Dispatch<DataMessage>;
  datasets: readonly Api.Dataset[];
  label: (dataset: Api.Dataset) => string;
}

function RenamePanel({ project, doc, dispatch, datasets, label }: DocumentProps) {
  const { batch, start } = useBatch();
  const changes = tagChanges(doc, datasets);
  const byId = new Map(datasets.map((d) => [d.id, d]));
  const commit = async () => {
    const done = await start(
      changes,
      (c) => {
        const d = byId.get(c.id);
        return d ? `${label(d)} → [${c.tags.join(" ")}]` : String(c.id);
      },
      (c) => api.command("dataset.tag", { project, id: c.id, tags: c.tags }),
    );
    dispatch({ type: "renamed", changes: done });
    await queryClient.invalidateQueries({ queryKey: keys.datasets(project) });
  };
  return (
    <div className="flex flex-col gap-[15px]">
      <p>
        <T k="data_tag_manual_change" />
      </p>
      <AmButton block k="data_update_name_btn" disabled={changes.length === 0 || batch.state === "running"} onClick={() => void commit()} />
      <BatchStatus batch={batch} doneKey="data.renamed" />
    </div>
  );
}

function DeletePanel({ project, selected: chosen, dispatch, datasets, runs, label }: DocumentProps & { runs: readonly Api.RunRecord[] }) {
  const { t } = useI18n();
  const { batch, start } = useBatch();
  const [confirming, setConfirming] = useState(false);
  const selected = selectedDatasets(chosen, datasets);
  const chosenRuns = runs.filter((r) => selectedRuns(chosen).includes(r.id));
  const runLabel = (r: Api.RunRecord) => `${t("data.kind_job")} #${r.id} ${analysisLabel(t, r.analysis)} [${r.tags.join(" ")}]`;
  // Datasets first, then jobs: a job keeps its outputs that are not selected.
  type Item = { dataset: Api.Dataset } | { run: Api.RunRecord };
  const items: Item[] = [...selected.map((dataset) => ({ dataset })), ...chosenRuns.map((run) => ({ run }))];
  const itemLabel = (i: Item) => ("dataset" in i ? label(i.dataset) : runLabel(i.run));
  const remove = async () => {
    const done = await start(items, itemLabel, (i) =>
      "dataset" in i ? api.command("dataset.remove", { project, id: i.dataset.id }) : api.command("run.remove", { project, id: i.run.id, outputs: false }),
    );
    dispatch({
      type: "removed",
      ids: done.flatMap((i) => ("dataset" in i ? [i.dataset.id] : [])),
      runs: done.flatMap((i) => ("run" in i ? [i.run.id] : [])),
    });
    await queryClient.invalidateQueries({ queryKey: keys.datasets(project) });
    await queryClient.invalidateQueries({ queryKey: keys.runs(project) });
  };
  return (
    <div className="flex flex-col gap-[15px]">
      {items.length === 0 ? (
        <p className="text-muted-foreground">
          <T k="data.selected_none" />
        </p>
      ) : (
        <ul className="list-inside list-disc">
          {items.map((i) => (
            <li key={"dataset" in i ? `d${i.dataset.id}` : `j${i.run.id}`}>{itemLabel(i)}</li>
          ))}
        </ul>
      )}
      {chosenRuns.length > 0 && <p className="text-xs text-muted-foreground">{t("data.remove_jobs_note")}</p>}
      <AmButton
        variant="danger"
        block
        k="data_delete_permanently"
        disabled={items.length === 0 || batch.state === "running"}
        onClick={() => setConfirming(true)}
      />
      <BatchStatus batch={batch} doneKey="data.removed" />
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} confirm="data_delete_permanently" onConfirm={() => void remove()}>
        <p className="mb-2">
          <T k="data_delete_warning" />
        </p>
        <ul className="list-inside list-disc">
          {items.map((i) => (
            <li key={"dataset" in i ? `d${i.dataset.id}` : `j${i.run.id}`}>{itemLabel(i)}</li>
          ))}
        </ul>
      </ConfirmDialog>
    </div>
  );
}
