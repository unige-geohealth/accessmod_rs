// Facilities of an analysis: the layer and identifier inputs, and the AM5
// "Facilities selection" table (paged, filter, checks, land cover at each
// facility).
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { MessageList } from "@accessmod/ui/am/messages";
import { type Kind, kindOf, match, NO_QUERY, type Query, type QueryColumn, SelectionToolbar } from "@accessmod/ui/am/selection-toolbar";
import { CenterTitle, SubTitle } from "@accessmod/ui/am/section";
import { type Key, T, useI18n } from "@accessmod/ui/i18n";
import { useMemo, useState } from "react";
import { api, issuesOf } from "../../api/client";
import { useRows, useValues } from "../../api/queries";
import {
  AM_COLUMNS,
  checkedOnPage,
  checkPage,
  count,
  type FacilityContext,
  type FacilityInput,
  type FacilityMessage,
  facilityRow,
  selectMatching,
  unselectMatching,
} from "../../state/facility-selection";
import type { ScenarioRow } from "../../state/scenario";
import { useFmt } from "../common";
import { DatasetSelect, FieldSelect, LINK, type Options, Pager } from "./inputs";

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

/**
 * Every identifier matching the filter, page after page (bounded pages),
 * and those whose row (AccessMod columns included) meets `condition`.
 */
async function everyId(
  project: string,
  dataset: number,
  where: string,
  idField: string,
  sample: number[],
  context: FacilityContext,
  condition?: (row: Row) => boolean,
): Promise<{ all: unknown[]; matching: unknown[] }> {
  const all: unknown[] = [];
  const matching: unknown[] = [];
  for (let offset = 0; ; offset += 1000) {
    const page = await api.rows(project, dataset, { offset, limit: 1000, where, sample });
    const at = page.columns.indexOf(idField);
    page.rows.forEach((values, i) => {
      all.push(values[at]);
      if (condition?.(facilityRow(values, page.columns, page.samples?.[i], context))) matching.push(values[at]);
    });
    if (page.rows.length === 0 || offset + page.rows.length >= page.total) return { all, matching };
  }
}

/** Values of the yes / no AccessMod columns. */
const YES_NO = ["no", "yes"];
/** Distinct values of a text field at most for a value select (AM5). */
const MAX_CHOICES = 200;

export function FacilitySection({
  project,
  input,
  landCover,
  population,
  scenario,
  onMessage,
  group,
  titled = true,
}: {
  project: string;
  input: FacilityInput;
  landCover: number | null;
  /** Population raster: adds AM5 `amPopCell`, the population of the cell of each facility. */
  population?: number | null;
  scenario: readonly ScenarioRow[];
  onMessage: (message: FacilityMessage) => void;
  /** Group heading, e.g. AM5 Referral "From" and "To". */
  group?: Key;
  /** With the "Facilities selection" titles (the first table of a tab). */
  titled?: boolean;
}) {
  const { layer: facilities, idField, selection } = input;
  const onSelection = (next: FacilityInput["selection"]) => onMessage({ type: "selection", selection: next });
  const { t } = useI18n();
  const f = useFmt();
  const [offset, setOffset] = useState(0);
  const [query, setQuery] = useState<Query>(NO_QUERY);
  const [failure, setFailure] = useState<string[]>([]);
  const sample = [landCover, population ?? null].filter((id) => id !== null);
  const page = useRows(project, facilities, { offset, limit: PAGE, where: selection.where, sample });
  const data = page.data;
  const at = data?.columns.indexOf(idField) ?? -1;
  const ids = useMemo(() => (data && at >= 0 ? data.rows.map((r) => r[at]) : []), [data, at]);
  const context = useMemo<FacilityContext>(
    () => ({
      landCover: landCover !== null,
      popAt: population === undefined ? undefined : population === null ? -1 : landCover === null ? 0 : 1,
      speeds: new Map(scenario.map((r) => [r.class, r.speed])),
    }),
    [landCover, population, scenario],
  );
  const rows = useMemo(() => (data ? data.rows.map((values, i) => facilityRow(values, data.columns, data.samples?.[i], context)) : []), [data, context]);
  const columns: Column[] = useMemo(() => {
    const rest = (data?.columns ?? []).filter((c) => c !== idField).map((field) => ({ field }));
    const pop = population === undefined ? [] : ["amPopCell"];
    // The id and the AccessMod columns stay in view; the layer's own fields scroll (AM5).
    const fixed = [idField, ...AM_COLUMNS, ...pop].map((field) => ({ field, frozen: true }));
    return [...fixed, ...rest];
  }, [data?.columns, idField, population]);
  // The value select of a text field of the layer: its distinct values, all pages.
  const layerFields = data?.columns ?? [];
  const chosen = columns.find((c) => c.field === query.field)?.field ?? columns[0]?.field ?? "";
  const chosenKind: Kind = kindOf(rows, chosen);
  const valuesOf = useValues(project, facilities, layerFields.includes(chosen) && chosenKind === "text" ? chosen : "", "");
  const queryColumns: QueryColumn[] = useMemo(
    () =>
      columns.map(({ field }) => {
        const kind = kindOf(rows, field);
        if (field === "amOnBarrier" || field === "amOnZero") return { field, kind, values: YES_NO };
        const known = field === chosen && valuesOf.data?.complete && valuesOf.data.values.length <= MAX_CHOICES;
        const values = known ? valuesOf.data?.values.flatMap((v) => (v.value === null ? [] : [String(v.value)])) : undefined;
        return { field, kind, values };
      }),
    [columns, rows, chosen, valuesOf.data],
  );

  if (facilities === null) {
    return (
      <>
        {titled && <CenterTitle title="analysis_facility_selection_title" sub="analysis_facility_selection_sub" />}
        <div className="px-[15px]">
          {group && <GroupTitle k={group} />}
          <MessageList messages={[{ kind: "info", k: "analysis_facility_selection_add_data" }]} />
        </div>
      </>
    );
  }

  const total = data?.total ?? 0;
  const changePage = async (checked: Row[]) => {
    setFailure([]);
    try {
      const all = selection.mode === "all" ? (await everyId(project, facilities, selection.where, idField, sample, context)).all : undefined;
      onSelection(
        checkPage(
          selection,
          ids,
          checked.map((r) => r[idField]),
          all,
        ),
      );
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };
  const selectWhere = async ({ field, op, value }: Query, kind: Kind, select: boolean) => {
    setFailure([]);
    try {
      const found = await everyId(project, facilities, selection.where, idField, sample, context, (row) => match(row[field], op, value, kind));
      onSelection((select ? selectMatching : unselectMatching)(selection, found.matching, found.all));
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };

  return (
    <>
      {titled && <CenterTitle title="analysis_facility_selection_title" sub="analysis_facility_selection_sub" />}
      <div className="flex flex-col gap-2 px-[15px]">
        {titled && <SubTitle k="analysis_facility_selected_facilities" />}
        {group && <GroupTitle k={group} />}
        {selection.where.trim() !== "" && (
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span>{f("analysis.job_filter", selection.where)}</span>
            <button
              type="button"
              className={LINK}
              onClick={() => {
                setOffset(0);
                onSelection({ mode: "all", where: "" });
              }}
            >
              [{t("analysis.clear_filter")}]
            </button>
          </div>
        )}
        {page.isError && <MessageList messages={issuesOf(page.error, t).map((text) => ({ kind: "issue", text }))} />}
        {failure.length > 0 && <MessageList messages={failure.map((text) => ({ kind: "issue", text }))} />}
        {data && total === 0 ? (
          <MessageList messages={[{ kind: "info", k: "analysis.no_rows" }]} />
        ) : (
          <DataTable
            toolbar={
              <SelectionToolbar
                columns={queryColumns}
                query={query}
                onQuery={setQuery}
                onAll={() => onSelection({ mode: "all", where: selection.where })}
                onNone={() => onSelection({ mode: "ids", where: selection.where, ids: [] })}
                onMatch={(q, kind, select) => void selectWhere(q, kind, select)}
                count={f("analysis.facilities_selected", count(selection, total), total)}
              />
            }
            selectable
            rowId={idField}
            selected={checkedOnPage(selection, ids)}
            onSelectionChange={(checked) => void changePage(checked)}
            columns={columns}
            data={rows}
            height="420px"
          />
        )}
        <Pager offset={offset} page={PAGE} total={total} onOffset={setOffset} />
      </div>
    </>
  );
}

function GroupTitle({ k }: { k: Key }) {
  return (
    <h3 className="text-xl">
      <T k={k} />
    </h3>
  );
}
