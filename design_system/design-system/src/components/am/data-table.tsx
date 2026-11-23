import { type ColumnDefinition, TabulatorFull as Tabulator } from "tabulator-tables";
import "tabulator-tables/dist/css/tabulator.min.css";
import "./data-table.css";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "#ui/i18n";
import { distinctValues, kindOf, match, NO_QUERY, type QueryColumn, SelectionToolbar } from "#ui/components/am/selection-toolbar";

export type Row = Record<string, string | number | boolean | null>;
export type Column = {
  field: string;
  title?: string;
  /** A text editor, or the given one: a number, or a choice among `values`. */
  editable?: boolean | "number" | { values: string[] };
  /** Rows whose cell of an `editable` column can be edited; all when absent. */
  editableRow?: (row: Row) => boolean;
  width?: number;
  /** Kept on the left while the other columns scroll (AM5: the AccessMod columns). */
  frozen?: boolean;
};

type DataTableProps = {
  columns: Column[];
  data: Row[];
  /** CSS height; AM5 tables are fixed height with an internal scroll. */
  height?: string;
  /** Checkbox column and the AM5 selection toolbar. */
  selectable?: boolean;
  onSelectionChange?: (rows: Row[]) => void;
  /** Field identifying a row; needed for a controlled `selected`. */
  rowId?: string;
  /**
   * Controlled selection, as `rowId` values: the owner keeps it in its state
   * and gets user changes through `onSelectionChange`.
   */
  selected?: (string | number)[];
  /** A cell of an `editable` column was edited by the user. */
  onCellEdited?: (row: Row, field: string, value: unknown) => void;
  /** `fitData` sizes columns to content (zonal statistics); default fills the width. */
  layout?: "fitColumns" | "fitData";
  /** Adds the AM5 export bar ([Download CSV] [Download JSON]); file name without extension. */
  exportName?: string;
  /**
   * The AM5 selection toolbar of a `selectable` table over its rows (default),
   * none (`false`), or the owner's, in its place (rows paged on a server).
   */
  toolbar?: boolean | ReactNode;
};

/** Header cell padding and sort arrow, around the title text. */
const HEADER_CHROME = 36;
let measure: CanvasRenderingContext2D | null = null;

/**
 * Width of a header title (bold 12 px): the column never gets narrower, so
 * every header stays readable; past the table width it scrolls.
 */
function headerWidth(title: string, host: HTMLElement): number {
  measure ??= document.createElement("canvas").getContext("2d");
  if (!measure) return 8 * title.length + HEADER_CHROME;
  measure.font = `700 12px ${getComputedStyle(host).fontFamily}`;
  return Math.ceil(measure.measureText(title).width) + HEADER_CHROME;
}

function editorOf(editable: Column["editable"]): Pick<ColumnDefinition, "editor" | "editorParams"> {
  if (editable === true) return { editor: "input" };
  if (editable === "number") return { editor: "number", editorParams: { min: 0 } };
  if (editable) return { editor: "list", editorParams: { values: editable.values } };
  return {};
}

/**
 * AM5 `amtabulator`: Tabulator with sortable headers, striped rows, square
 * frame, optional selection toolbar. Imperative widget behind a React
 * adapter: React owns the data, Tabulator only draws it.
 */
export function DataTable({
  columns,
  data,
  height = "400px",
  selectable,
  onSelectionChange,
  rowId,
  selected: selectedIds,
  onCellEdited,
  layout = "fitColumns",
  exportName,
  toolbar = true,
}: DataTableProps) {
  const { t } = useI18n();
  const el = useRef<HTMLDivElement>(null);
  /** Set once Tabulator has finished its (async) build. */
  const table = useRef<Tabulator | null>(null);
  const dataRef = useRef(data);
  dataRef.current = data;
  const selectedRef = useRef(selectedIds);
  selectedRef.current = selectedIds;
  const onSelectionRef = useRef(onSelectionChange);
  onSelectionRef.current = onSelectionChange;
  const onEditedRef = useRef(onCellEdited);
  onEditedRef.current = onCellEdited;
  /** Above zero while rows or selection change from props: no change is emitted. */
  const syncing = useRef(0);
  const [selected, setSelected] = useState(0);
  const [query, setQuery] = useState(NO_QUERY);
  const queryColumns = useMemo<QueryColumn[]>(
    () =>
      columns.map((c) => {
        const kind = kindOf(data, c.field);
        return { field: c.field, title: c.title, kind, values: kind === "text" ? distinctValues(data, c.field) : undefined };
      }),
    [columns, data],
  );

  /** Make Tabulator's selection match the controlled one. */
  const syncSelection = (tab: Tabulator) => {
    const ids = selectedRef.current;
    if (!rowId || !ids) return;
    const want = new Set(ids.map(String));
    syncing.current++;
    try {
      for (const r of tab.getRows()) {
        const on = want.has(String(r.getData()[rowId]));
        if (on !== r.isSelected()) on ? r.select() : r.deselect();
      }
    } finally {
      syncing.current--;
    }
    setSelected(tab.getSelectedRows().length);
  };

  /** New rows from props, then the controlled selection again. */
  const replaceRows = (tab: Tabulator, rows: Row[]) => {
    syncing.current++;
    void tab.replaceData(rows).finally(() => {
      syncing.current--;
      syncSelection(tab);
    });
  };

  useEffect(() => {
    const host = el.current;
    if (!host) return;
    // Own node per instance: a StrictMode remount may start a second table
    // while the first one is still building.
    const node = host.appendChild(document.createElement("div"));
    let built = false;
    let disposed = false;
    const dispose = (tab: Tabulator) => {
      tab.destroy();
      node.remove();
    };
    const defs: ColumnDefinition[] = columns.map((c) => ({
      field: c.field,
      title: c.title ?? c.field,
      ...editorOf(c.editable),
      ...(c.editableRow ? { editable: (cell: { getRow: () => { getData: () => Row } }) => c.editableRow!(cell.getRow().getData()) } : {}),
      width: c.width,
      minWidth: headerWidth(c.title ?? c.field, host),
      frozen: c.frozen,
      headerSort: true,
    }));
    if (selectable) {
      defs.unshift({ title: "", formatter: "rowSelection", titleFormatter: "rowSelection", hozAlign: "center", headerSort: false, width: 40, frozen: true });
    }
    const initial = dataRef.current;
    const tab = new Tabulator(node, {
      data: initial,
      columns: defs,
      height,
      layout,
      selectableRows: !!selectable,
      index: rowId ?? "id",
    });
    tab.on("tableBuilt", () => {
      built = true;
      if (disposed) return dispose(tab);
      table.current = tab;
      if (dataRef.current !== initial) replaceRows(tab, dataRef.current);
      else syncSelection(tab);
    });
    tab.on("rowSelectionChanged", (rows: Row[]) => {
      setSelected(rows.length);
      if (syncing.current === 0) onSelectionRef.current?.(rows);
    });
    tab.on("cellEdited", (cell) => {
      onEditedRef.current?.(cell.getRow().getData() as Row, cell.getField(), cell.getValue());
    });
    return () => {
      disposed = true;
      table.current = null;
      if (built) dispose(tab);
      else node.hidden = true; // disposed on tableBuilt
    };
    // the column content (not the array identity) defines the table;
    // data updates go through replaceData below
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [JSON.stringify(columns), height, selectable, layout, rowId]);

  useEffect(() => {
    if (table.current) replaceRows(table.current, data);
    // replaceRows reads refs only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  useEffect(() => {
    if (table.current) syncSelection(table.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedIds]);

  return (
    <div className="am-table border border-table-border">
      {selectable && toolbar !== true && toolbar}
      {selectable && toolbar === true && (
        <SelectionToolbar
          columns={queryColumns}
          query={query}
          onQuery={setQuery}
          count={`(${selected}/${data.length})`}
          onAll={() => table.current?.selectRow()}
          onNone={() => table.current?.deselectRow()}
          onMatch={({ field, op, value }, kind, select) => {
            const rows = table.current?.getRows().filter((r) => match(r.getData()[field], op, value, kind)) ?? [];
            for (const r of rows) select ? r.select() : r.deselect();
          }}
        />
      )}
      <div ref={el} />
      {exportName && (
        <div className="flex gap-4 border-t border-table-border bg-table-toolbar px-3 py-1.5">
          {(["csv", "json"] as const).map((type) => (
            <button
              key={type}
              type="button"
              className="text-link hover:text-link-hover hover:underline"
              onClick={() => table.current?.download(type, `${exportName}.${type}`)}
            >
              [{t(type === "csv" ? "table.download_csv" : "table.download_json")}]
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
