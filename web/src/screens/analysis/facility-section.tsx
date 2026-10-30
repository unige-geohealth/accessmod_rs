// Facilities of an analysis: the layer and identifier inputs, and the AM5
// "Facilities selection" table (paged, filter, checks, land cover at each
// facility).
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { TextField } from "@accessmod/ui/am/form";
import { AmButton, MessageList } from "@accessmod/ui/am/messages";
import { CenterTitle, SubTitle } from "@accessmod/ui/am/section";
import { type Key, useI18n } from "@accessmod/ui/i18n";
import { useMemo, useState } from "react";
import { api, issuesOf } from "../../api/client";
import { useRows } from "../../api/queries";
import { checkedOnPage, checkPage, count, type FacilityInput, type FacilityMessage } from "../../state/facility-selection";
import type { ScenarioRow } from "../../state/scenario";
import { useFmt } from "../common";
import { DatasetSelect, FieldSelect, type Options } from "./inputs";

/** Data input: the facility layer and its identifier field. */
export function FacilityInputs({
  label,
  idLabel,
  options,
  input,
  fields,
  onMessage,
}: {
  label: Key;
  idLabel: Key;
  options: Options;
  input: FacilityInput;
  fields: readonly string[];
  onMessage: (message: FacilityMessage) => void;
}) {
  return (
    <>
      <DatasetSelect label={label} options={options} value={input.layer} onChange={(id) => onMessage({ type: "layer", id })} />
      <FieldSelect label={idLabel} fields={fields} value={input.idField} onChange={(field) => onMessage({ type: "idField", field })} />
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

export function FacilitySection({
  project,
  input,
  landCover,
  scenario,
  onMessage,
}: {
  project: string;
  input: FacilityInput;
  landCover: number | null;
  scenario: readonly ScenarioRow[];
  onMessage: (message: FacilityMessage) => void;
}) {
  const { layer: facilities, idField, selection } = input;
  const onSelection = (next: FacilityInput["selection"]) => onMessage({ type: "selection", selection: next });
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
