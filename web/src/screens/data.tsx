// Data (AM5 `amManageData`): import, filter, rename, archive and delete on
// the left; "Available data" on the right. The table's selection and tag
// edits are the screen's document (state/data-document.ts); filters are in
// the URL; datasets come from the server.
import { Columns } from "@accessmod/ui/am/columns";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { DataTable, type Row } from "@accessmod/ui/am/data-table";
import { RadioField, SelectField, TextField } from "@accessmod/ui/am/form";
import { AmButton, FilePicker, InfoList, MessageList } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { CenterTitle } from "@accessmod/ui/am/section";
import { T, useI18n } from "@accessmod/ui/i18n";
import { type Dispatch, useMemo, useReducer, useState } from "react";
import { api, issuesOf } from "../api/client";
import { keys, queryClient, useDatasets } from "../api/queries";
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
  kindOf,
  matches,
  normalizeTags,
  sortRows,
  toRow,
} from "../state/catalogue";
import { type DataDocument, type DataMessage, EMPTY, selectedDatasets, tagChanges, updateData } from "../state/data-document";
import { busy, useTransfer } from "../state/transfer";
import { BatchStatus, NotAvailable, TransferStatus, useFmt } from "./common";

export function DataScreen({ route, project }: { route: Route; project: string }) {
  const { t } = useI18n();
  const datasets = useDatasets(project);
  const [doc, dispatch] = useReducer(updateData, EMPTY);
  const filter = filterFromQuery(route.query);
  const renaming = route.panel === "rename";
  const label = useMemo(() => (dataset: Api.Dataset) => `${classLabel(t, dataset.class)} [${dataset.tags.join(" ")}]`, [t]);

  const list = datasets.data;
  const rows = useMemo(() => {
    const all = sortRows((list ?? []).map((d) => toRow(d, t)));
    return all.filter((r) => matches(r, filter)).map((r) => ({ ...r, tags: doc.edits[r.id] ?? r.tags }));
    // the filter is a value of the URL query
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list, t, doc.edits, filter.type, filter.text, filter.tags]);

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
            { value: "filter", icon: "filter", title: "data_filter_data", content: <FilterPanel filter={filter} onChange={setFilter} /> },
            {
              value: "rename",
              icon: "rename",
              title: "data_rename_data",
              content: <RenamePanel project={project} doc={doc} dispatch={dispatch} datasets={list ?? []} label={label} />,
            },
            {
              value: "archive",
              icon: "archive",
              title: "data_archive_data",
              content: (
                <div className="flex flex-col gap-[15px]">
                  <AmButton block disabled k="data_archive_create" />
                  <NotAvailable />
                </div>
              ),
            },
            {
              value: "delete",
              icon: "delete",
              title: "data_delete_data",
              content: <DeletePanel project={project} doc={doc} dispatch={dispatch} datasets={list ?? []} label={label} />,
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
            rowId="id"
            selected={doc.selected as number[]}
            onSelectionChange={(selected: Row[]) => dispatch({ type: "select", ids: selected.map((r) => Number(r.id)) })}
            onCellEdited={(row, field, value) => {
              if (field === "tags") dispatch({ type: "editTags", id: Number(row.id), text: String(value ?? "") });
            }}
            columns={[
              { field: "tags", title: t("data.column_tags"), editable: renaming },
              { field: "type", title: t("data.column_type") },
              { field: "class", title: t("data.column_class") },
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
    void start(files, async (upload) => {
      const record = await api.command("dataset.import", { project, source: { upload: { id: upload.id } }, class: cls, tags });
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
        multiple={cls !== undefined && kindOf(cls) === "vector"}
        disabled={!ready}
        onFiles={importFiles}
      />
      <TransferStatus transfer={state} doneKey="data.imported" />
    </div>
  );
}

function FilterPanel({ filter, onChange }: { filter: DataFilter; onChange: (f: DataFilter) => void }) {
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
          { value: "config", label: "data_config" },
          { value: "all", label: "data_all" },
        ]}
      />
      <TextField label="data_filter_text" value={filter.text} onChange={(text) => onChange({ ...filter, text })} />
      <TextField label="data_filter_tag" value={filter.tags} onChange={(tags) => onChange({ ...filter, tags })} />
    </div>
  );
}

interface DocumentProps {
  project: string;
  doc: DataDocument;
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

function DeletePanel({ project, doc, dispatch, datasets, label }: DocumentProps) {
  const { batch, start } = useBatch();
  const [confirming, setConfirming] = useState(false);
  const selected = selectedDatasets(doc, datasets);
  const remove = async () => {
    const done = await start(selected, label, (d) => api.command("dataset.remove", { project, id: d.id }));
    dispatch({ type: "removed", ids: done.map((d) => d.id) });
    await queryClient.invalidateQueries({ queryKey: keys.datasets(project) });
  };
  return (
    <div className="flex flex-col gap-[15px]">
      {selected.length === 0 ? (
        <p className="text-muted-foreground">
          <T k="data.selected_none" />
        </p>
      ) : (
        <ul className="list-inside list-disc">
          {selected.map((d) => (
            <li key={d.id}>{label(d)}</li>
          ))}
        </ul>
      )}
      <AmButton
        variant="danger"
        block
        k="data_delete_permanently"
        disabled={selected.length === 0 || batch.state === "running"}
        onClick={() => setConfirming(true)}
      />
      <BatchStatus batch={batch} doneKey="data.removed" />
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} confirm="data_delete_permanently" onConfirm={() => void remove()}>
        <p className="mb-2">
          <T k="data_delete_warning" />
        </p>
        <ul className="list-inside list-disc">
          {selected.map((d) => (
            <li key={d.id}>{label(d)}</li>
          ))}
        </ul>
      </ConfirmDialog>
    </div>
  );
}
