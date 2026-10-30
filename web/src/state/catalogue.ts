// Catalogue selectors: data classes, tags, the Data › Filter rules and the
// rows of the "Available data" table. Pure functions of server data and
// view state; nothing here is stored.
import type { Key } from "@accessmod/ui/i18n";
import classList from "../../schemas/data-classes.json";
import type { ClassInfo } from "../api/client";
import type { Api } from "../generated/types";

export type Kind = ClassInfo["kind"];

/** Every data class, in the order of the registry (`cargo xtask schemas`). */
export const CLASSES = classList as ClassInfo[];

const byCode = new Map(CLASSES.map((c) => [c.class, c]));

export const classInfo = (code: Api.DataClass): ClassInfo | undefined => byCode.get(code);

export const kindOf = (code: Api.DataClass): Kind => classInfo(code)?.kind ?? "table";

/** Translated class label; the registry label when the catalogue has none. */
export function classLabel(t: (key: Key) => string, code: Api.DataClass): string {
  const key = `class.${code}` as Key;
  const text = t(key);
  return text === key ? (classInfo(code)?.label ?? code) : text;
}

/**
 * Files the picker offers for a class, by kind, as the server reads uploads:
 * a shapefile comes with its companions, an ERDAS Imagine raster with its
 * spill (`.ige`) and overview (`.rrd`) files.
 */
export const ACCEPT: Record<Kind, string> = {
  raster: ".tif,.tiff,.img,.ige,.rrd",
  vector: ".shp,.shx,.dbf,.prj,.cpg,.gpkg,.fgb,.geojson,.json",
  table: ".csv,.xlsx",
  config: ".json",
};

/** Upper and lower case Latin letters spelled out, as the server's `normalize_tags`. */
const SPELLED: Record<string, string> = {
  ß: "ss",
  ẞ: "SS",
  æ: "ae",
  Æ: "AE",
  œ: "oe",
  Œ: "OE",
  ø: "o",
  Ø: "O",
  đ: "d",
  ð: "d",
  Đ: "D",
  Ð: "D",
  ł: "l",
  Ł: "L",
  þ: "th",
  Þ: "TH",
  ı: "i",
};

/**
 * Tags as the server keeps them (`accessmod_io::project::normalize_tags`,
 * AM5 `amGetUniqueTags`): Latin letters to ASCII, any character but letters
 * and digits separates, duplicates removed, order kept. A preview only: the
 * server normalises again.
 */
export function normalizeTags(text: string): string[] {
  const ascii = Array.from(text.normalize("NFD").replace(/\p{M}/gu, ""), (c) => SPELLED[c] ?? c).join("");
  const out: string[] = [];
  // Rust's `char::is_alphanumeric`: Unicode Alphabetic or Numeric.
  for (const word of ascii.split(/[^\p{Alphabetic}\p{N}]+/u)) {
    if (word !== "" && !out.includes(word)) out.push(word);
  }
  return out;
}

/** Data › Filter, from the URL. */
export interface DataFilter {
  type: Kind | "all";
  /** Any field, case sensitive (AM5). */
  text: string;
  /** Every tag must be present. */
  tags: string;
  /** Only the datasets of the latest finished run ("Filter last results"). */
  last: boolean;
}

const KINDS: readonly string[] = ["vector", "raster", "table", "config"];

export function filterFromQuery(query: Readonly<Record<string, string>>): DataFilter {
  const type = query.type ?? "all";
  return {
    type: KINDS.includes(type) ? (type as Kind) : "all",
    text: query.text ?? "",
    tags: query.tags ?? "",
    last: query.last === "1",
  };
}

export function filterToQuery(filter: DataFilter): Record<string, string> {
  return { type: filter.type === "all" ? "" : filter.type, text: filter.text, tags: filter.tags, last: filter.last ? "1" : "" };
}

/** AM5 names of the kinds (Data › Filter). */
export const KIND_KEYS: Record<Kind, Key> = { vector: "data_vector", raster: "data_raster", table: "data_table", config: "data_config" };

/** A row of the "Available data" table (AM5: Tags, Type, Class). */
export interface DataRow {
  [field: string]: string | number;
  id: number;
  tags: string;
  kind: Kind;
  /** The kind, translated. */
  type: string;
  class: string;
}

export function toRow(dataset: Api.Dataset, t: (key: Key) => string): DataRow {
  const kind = kindOf(dataset.class);
  return { id: dataset.id, tags: dataset.tags.join(" "), kind, type: t(KIND_KEYS[kind]), class: classLabel(t, dataset.class) };
}

/** Whether `row` passes `filter`; `last` holds the datasets of the latest finished run. */
export function matches(row: DataRow, filter: DataFilter, last: ReadonlySet<number> = new Set()): boolean {
  if (filter.last && !last.has(row.id)) return false;
  if (filter.type !== "all" && row.kind !== filter.type) return false;
  if (filter.text !== "" && ![row.tags, row.type, row.class].some((f) => f.includes(filter.text))) return false;
  const wanted = normalizeTags(filter.tags);
  if (wanted.length > 0) {
    const tags = row.tags.split(" ");
    if (!wanted.every((w) => tags.includes(w))) return false;
  }
  return true;
}

/** Datasets sorted as AM5 lists them: by class label, then tags. */
export function sortRows(rows: DataRow[]): DataRow[] {
  return [...rows].sort((a, b) => a.class.localeCompare(b.class) || a.tags.localeCompare(b.tags) || a.id - b.id);
}

/** Classes a job field accepts: its `x-accessmod-class` in the job schema. */
export function fieldClasses(schema: unknown, field: string): Api.DataClass[] {
  const properties = (schema as { properties?: Record<string, Record<string, unknown>> }).properties;
  const classes = properties?.[field]?.["x-accessmod-class"];
  return Array.isArray(classes) ? (classes as Api.DataClass[]) : [];
}

/** AM5 data selector label: "(r) land cover merged [demo]". */
export function datasetLabel(t: (key: Key) => string, dataset: Api.Dataset): string {
  return `(${kindOf(dataset.class).charAt(0)}) ${classLabel(t, dataset.class)} [${dataset.tags.join(" ")}]`;
}

/** Datasets of `classes`, newest first, as select options (value: id). */
export function datasetOptions(
  t: (key: Key) => string,
  datasets: readonly Api.Dataset[] | undefined,
  classes: readonly Api.DataClass[],
): { value: string; label: string }[] {
  return (datasets ?? [])
    .filter((d) => classes.includes(d.class))
    .sort((a, b) => b.id - a.id)
    .map((d) => ({ value: String(d.id), label: datasetLabel(t, d) }));
}

/** The table of `classes` with exactly the tags of `dataset` (AM5 pairs layers by tags), e.g. land cover labels. */
export function pairedTable(datasets: readonly Api.Dataset[] | undefined, dataset: Api.Dataset | undefined, cls: Api.DataClass): Api.Dataset | undefined {
  if (dataset === undefined) return undefined;
  const tags = [...dataset.tags].sort().join(" ");
  return (datasets ?? []).filter((d) => d.class === cls && [...d.tags].sort().join(" ") === tags).sort((a, b) => b.id - a.id)[0];
}
