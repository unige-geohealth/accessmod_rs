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

/** Files the picker offers for a class, by kind; a shapefile comes with its companions. */
export const ACCEPT: Record<Kind, string> = {
  raster: ".tif,.tiff,.img,.vrt",
  vector: ".shp,.shx,.dbf,.prj,.cpg,.gpkg,.fgb,.geojson,.json,.sqlite",
  table: ".csv",
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
  for (const word of ascii.split(/[^\p{L}\p{N}]+/u)) {
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
}

const KINDS: readonly string[] = ["vector", "raster", "table", "config"];

export function filterFromQuery(query: Readonly<Record<string, string>>): DataFilter {
  const type = query.type ?? "all";
  return {
    type: KINDS.includes(type) ? (type as Kind) : "all",
    text: query.text ?? "",
    tags: query.tags ?? "",
  };
}

export function filterToQuery(filter: DataFilter): Record<string, string> {
  return { type: filter.type === "all" ? "" : filter.type, text: filter.text, tags: filter.tags };
}

/** A row of the "Available data" table (AM5: Tags, Type, Class). */
export interface DataRow {
  [field: string]: string | number;
  id: number;
  tags: string;
  type: Kind;
  class: string;
}

export function toRow(dataset: Api.Dataset, label: (code: Api.DataClass) => string): DataRow {
  return { id: dataset.id, tags: dataset.tags.join(" "), type: kindOf(dataset.class), class: label(dataset.class) };
}

export function matches(row: DataRow, filter: DataFilter): boolean {
  if (filter.type !== "all" && row.type !== filter.type) return false;
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
