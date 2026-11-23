// AM5 selection toolbar (amtabulator `src/TabulatorToolbar.js`):
// [All] [None] <column> <operator> <value> [Select] [Unselect] (n/N).
// It checks or unchecks the rows matching a condition; it does not filter.
import type { ReactNode } from "react";
import { useI18n } from "#ui/i18n";

export type Kind = "number" | "text";
export type Op = "=" | "!=" | ">" | "<" | ">=" | "<=" | "like";

/** Operators of a column, as AM5: comparisons for numbers, `like` for text. */
export function opsFor(kind: Kind): readonly Op[] {
  return kind === "number" ? ["=", "!=", ">", "<", ">=", "<="] : ["=", "!=", "like"];
}

/** Kind of a column, as AM5: the type of its first value (here the first one set). */
export function kindOf(rows: readonly Record<string, unknown>[], field: string): Kind {
  for (const row of rows) {
    const v = row[field];
    if (v !== null && v !== undefined) return typeof v === "number" ? "number" : "text";
  }
  return "text";
}

/** Distinct values of a text column, sorted, when at most `max` (AM5: 200): its value select. */
export function distinctValues(rows: readonly Record<string, unknown>[], field: string, max = 200): string[] | undefined {
  const values = new Set<string>();
  for (const row of rows) {
    const v = row[field];
    if (v === null || v === undefined) continue;
    values.add(String(v));
    if (values.size > max) return undefined;
  }
  return [...values].sort();
}

/**
 * Whether cell `v` meets `op value` (Tabulator `searchRows`, behind AM5):
 * numbers compare as numbers, text as text; `like` is a case-insensitive
 * substring. An empty cell meets only `!=`.
 */
export function match(v: unknown, op: Op, value: string, kind: Kind): boolean {
  if (v === null || v === undefined || v === "") return op === "!=";
  if (op === "like") return String(v).toLowerCase().includes(value.toLowerCase());
  if (kind === "number") {
    const a = Number(v);
    const b = Number(value);
    if (!Number.isFinite(a) || !Number.isFinite(b)) return op === "!=";
    return compare(a, b, op);
  }
  return compare(String(v), value, op);
}

function compare<T extends number | string>(a: T, b: T, op: Exclude<Op, "like">): boolean {
  switch (op) {
    case "=":
      return a === b;
    case "!=":
      return a !== b;
    case ">":
      return a > b;
    case "<":
      return a < b;
    case ">=":
      return a >= b;
    case "<=":
      return a <= b;
  }
}

/** The condition being edited. */
export interface Query {
  field: string;
  op: Op;
  value: string;
}

export const NO_QUERY: Query = { field: "", op: "=", value: "" };

/** A column of the toolbar: its kind gives the operators; `values` a select (text, few values). */
export interface QueryColumn {
  field: string;
  title?: string;
  kind: Kind;
  values?: readonly string[];
}

/**
 * The condition as shown: the first column until one is chosen, `=` when the
 * operator does not apply to the column, the first value of a value select.
 * Empty `value` (or no column): [Select] and [Unselect] do nothing (AM5).
 */
export function shownQuery(query: Query, columns: readonly QueryColumn[]): { query: Query; column?: QueryColumn } {
  const column = columns.find((c) => c.field === query.field) ?? columns[0];
  if (!column) return { query: NO_QUERY };
  const op = opsFor(column.kind).includes(query.op) ? query.op : "=";
  const choices = op === "like" ? undefined : column.values;
  const first = choices?.[0];
  const value = first !== undefined && !choices?.includes(query.value) ? first : query.value;
  return { query: { field: column.field, op, value }, column };
}

/** The query after the user picked another column: the value belongs to the old one. */
export const withField = (query: Query, field: string): Query => (field === query.field ? query : { ...query, field, value: "" });

export function SelectionToolbar({
  columns,
  query,
  onQuery,
  onAll,
  onNone,
  onMatch,
  count,
}: {
  columns: readonly QueryColumn[];
  query: Query;
  onQuery: (query: Query) => void;
  onAll: () => void;
  onNone: () => void;
  /** Check (`select`) or uncheck the rows meeting the shown condition. */
  onMatch: (query: Query, kind: Kind, select: boolean) => void;
  /** Count of checked rows, e.g. `(3/10)`. */
  count: ReactNode;
}) {
  const { t } = useI18n();
  const { query: shown, column } = shownQuery(query, columns);
  const link = "px-2 text-link hover:text-link-hover hover:underline";
  const control = "h-7 border border-input bg-input-background px-2 text-sm";
  const choices = shown.op === "like" ? undefined : column?.values;
  const run = (select: boolean) => {
    if (column && shown.value !== "") onMatch(shown, column.kind, select);
  };

  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-table-border bg-table-toolbar px-2.5 py-1.5">
      <button type="button" className={link} onClick={onAll}>
        [{t("table.all")}]
      </button>
      <button type="button" className={link} onClick={onNone}>
        [{t("table.none")}]
      </button>
      <select className={control} value={shown.field} onChange={(e) => onQuery(withField(shown, e.target.value))} aria-label={t("table.column")}>
        {columns.map((c) => (
          <option key={c.field} value={c.field}>
            {c.title ?? c.field}
          </option>
        ))}
      </select>
      <select className={control} value={shown.op} onChange={(e) => onQuery({ ...shown, op: e.target.value as Op })} aria-label={t("table.operator")}>
        {opsFor(column?.kind ?? "text").map((o) => (
          <option key={o}>{o}</option>
        ))}
      </select>
      {choices && choices.length > 0 ? (
        <select className={control} value={shown.value} onChange={(e) => onQuery({ ...shown, value: e.target.value })} aria-label={t("table.value")}>
          {choices.map((v) => (
            <option key={v}>{v}</option>
          ))}
        </select>
      ) : (
        <input
          className={`${control} w-32 min-w-0 flex-1`}
          type={column?.kind === "number" ? "number" : "text"}
          value={shown.value}
          onChange={(e) => onQuery({ ...shown, value: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === "Enter") run(true);
          }}
          aria-label={t("table.value")}
        />
      )}
      <button type="button" className={link} onClick={() => run(true)}>
        [{t("table.select")}]
      </button>
      <button type="button" className={link} onClick={() => run(false)}>
        [{t("table.unselect")}]
      </button>
      <span>{count}</span>
    </div>
  );
}
