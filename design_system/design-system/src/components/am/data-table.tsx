import { type ColumnDefinition, TabulatorFull as Tabulator } from "tabulator-tables";
import "tabulator-tables/dist/css/tabulator.min.css";
import "./data-table.css";
import { useEffect, useRef, useState } from "react";
import { useI18n } from "@/i18n";

export type Row = Record<string, string | number | boolean | null>;
export type Column = { field: string; title?: string; editable?: boolean; width?: number };

type DataTableProps = {
  columns: Column[];
  data: Row[];
  /** CSS height; AM5 tables are fixed height with an internal scroll. */
  height?: string;
  /** Checkbox column and the AM5 selection toolbar. */
  selectable?: boolean;
  onSelectionChange?: (rows: Row[]) => void;
  /** `fitData` sizes columns to content (zonal statistics); default fills the width. */
  layout?: "fitColumns" | "fitData";
  /** Adds the AM5 export bar ([Download CSV] [Download JSON]); file name without extension. */
  exportName?: string;
};

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
  layout = "fitColumns",
  exportName,
}: DataTableProps) {
  const { t } = useI18n();
  const el = useRef<HTMLDivElement>(null);
  /** Set once Tabulator has finished its (async) build. */
  const table = useRef<Tabulator | null>(null);
  const dataRef = useRef(data);
  dataRef.current = data;
  const [selected, setSelected] = useState(0);

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
      editor: c.editable ? "input" : undefined,
      width: c.width,
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
    });
    tab.on("tableBuilt", () => {
      built = true;
      if (disposed) return dispose(tab);
      table.current = tab;
      if (dataRef.current !== initial) tab.replaceData(dataRef.current);
    });
    tab.on("rowSelectionChanged", (rows: Row[]) => {
      setSelected(rows.length);
      onSelectionChange?.(rows);
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
  }, [JSON.stringify(columns), height, selectable, layout]);

  useEffect(() => {
    table.current?.replaceData(data);
  }, [data]);

  return (
    <div className="am-table border border-table-border">
      {selectable && (
        <SelectionToolbar
          columns={columns}
          selected={selected}
          total={data.length}
          onAll={() => table.current?.selectRow()}
          onNone={() => table.current?.deselectRow()}
          onMatch={(field, op, value, select) => {
            const rows = table.current?.getRows().filter((r) => match(r.getData()[field], op, value)) ?? [];
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

type Op = "=" | "!=" | ">" | "<" | "~";
const ops: Op[] = ["=", "!=", ">", "<", "~"];

function match(v: unknown, op: Op, value: string) {
  const s = String(v ?? "");
  const n = Number(v);
  switch (op) {
    case "=":
      return s === value;
    case "!=":
      return s !== value;
    case ">":
      return n > Number(value);
    case "<":
      return n < Number(value);
    case "~":
      return s.toLowerCase().includes(value.toLowerCase());
  }
}

function SelectionToolbar({
  columns,
  selected,
  total,
  onAll,
  onNone,
  onMatch,
}: {
  columns: Column[];
  selected: number;
  total: number;
  onAll: () => void;
  onNone: () => void;
  onMatch: (field: string, op: Op, value: string, select: boolean) => void;
}) {
  const { t } = useI18n();
  const [field, setField] = useState(columns[0]?.field ?? "");
  const [op, setOp] = useState<Op>("=");
  const [value, setValue] = useState("");
  const link = "px-2 text-link hover:text-link-hover hover:underline";
  const control = "h-7 border border-input bg-input-background px-2 text-sm";

  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-table-border bg-table-toolbar px-2.5 py-1.5">
      <button type="button" className={link} onClick={onAll}>
        [{t("table.all")}]
      </button>
      <button type="button" className={link} onClick={onNone}>
        [{t("table.none")}]
      </button>
      <select className={control} value={field} onChange={(e) => setField(e.target.value)} aria-label={t("table.column")}>
        {columns.map((c) => (
          <option key={c.field} value={c.field}>
            {c.title ?? c.field}
          </option>
        ))}
      </select>
      <select className={control} value={op} onChange={(e) => setOp(e.target.value as Op)} aria-label={t("table.operator")}>
        {ops.map((o) => (
          <option key={o}>{o}</option>
        ))}
      </select>
      <input className={`${control} w-32 min-w-0 flex-1`} value={value} onChange={(e) => setValue(e.target.value)} aria-label={t("table.value")} />
      <button type="button" className={link} onClick={() => onMatch(field, op, value, true)}>
        [{t("table.select")}]
      </button>
      <button type="button" className={link} onClick={() => onMatch(field, op, value, false)}>
        [{t("table.unselect")}]
      </button>
      <span>
        ({selected}/{total})
      </span>
    </div>
  );
}
