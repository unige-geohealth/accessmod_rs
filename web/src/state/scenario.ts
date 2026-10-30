// The scenario table of Analysis › Accessibility (AM5 "Scenario to
// process"): one row per class of the merged land cover, edited in place,
// optionally filled from a scenario table dataset. Pure functions; the
// rows are part of the analysis draft.
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";

/** AM5 transport modes, as its tables spell them. */
export const MODES = ["WALKING", "BICYCLING", "MOTORIZED"] as const;
export type Mode = (typeof MODES)[number];

/** AM5 default mode of a new row (`config$defaultTranspMode`). */
export const DEFAULT_MODE: Mode = "WALKING";

export interface ScenarioRow {
  class: number;
  label: string;
  /** km/h; 0 leaves the class out of the scenario: a barrier. */
  speed: number;
  mode: Mode;
}

export const isMode = (value: string): value is Mode => (MODES as readonly string[]).includes(value);

/** Integer classes of a land cover raster, from its stored statistics. */
export function landCoverClasses(dataset: Api.Dataset | undefined): number[] {
  if (dataset?.summary.kind !== "raster") return [];
  const values = dataset.summary.stats.values ?? [];
  return values.map((v) => v.value).filter((v) => Number.isInteger(v)).sort((a, b) => a - b);
}

/** Index of a column by name, case-insensitive; -1 when absent. */
function column(rows: Api.Rows, name: string): number {
  return rows.columns.findIndex((c) => c.toLowerCase() === name);
}

const text = (value: unknown): string => (value === null || value === undefined ? "" : String(value).trim());

function integer(value: unknown): number | null {
  const n = Number(text(value));
  return text(value) !== "" && Number.isInteger(n) ? n : null;
}

/** Labels by class from a land cover table (columns `class`, `label`). */
export function labelsOf(rows: Api.Rows | undefined): ReadonlyMap<number, string> {
  const labels = new Map<number, string>();
  if (rows === undefined) return labels;
  const [c, l] = [column(rows, "class"), column(rows, "label")];
  if (c < 0 || l < 0) return labels;
  for (const row of rows.rows) {
    const cls = integer(row[c]);
    if (cls !== null && !labels.has(cls)) labels.set(cls, text(row[l]));
  }
  return labels;
}

/** Rows of a land cover: AM5 initial table (no label: `no_label_<class>`; speed 0; walking). */
export function initialRows(classes: readonly number[], labels: ReadonlyMap<number, string>): ScenarioRow[] {
  return classes.map((cls) => ({
    class: cls,
    label: labels.get(cls) || `no_label_${cls}`,
    speed: 0,
    mode: DEFAULT_MODE,
  }));
}

/** Rows of a scenario table dataset (columns `class`, `label`, `speed`, `mode`), as stored; unreadable rows are skipped. */
export function tableRows(rows: Api.Rows | undefined): ScenarioRow[] {
  if (rows === undefined) return [];
  const [c, l, s, m] = ["class", "label", "speed", "mode"].map((name) => column(rows, name));
  if (c === undefined || l === undefined || s === undefined || m === undefined || c < 0) return [];
  const out: ScenarioRow[] = [];
  for (const row of rows.rows) {
    const cls = integer(row[c]);
    if (cls === null) continue;
    const speed = s < 0 ? 0 : Number(text(row[s]));
    const mode = m < 0 ? "" : text(row[m]).toUpperCase();
    out.push({
      class: cls,
      label: l < 0 ? "" : text(row[l]),
      speed: Number.isFinite(speed) && speed > 0 ? speed : 0,
      mode: isMode(mode) ? mode : DEFAULT_MODE,
    });
  }
  return out;
}

/**
 * AM5 "Import selected table": the table's rows for classes of the land
 * cover (first row of a repeated class), then the land cover classes the
 * table lacks, by class.
 */
export function mergeTable(current: readonly ScenarioRow[], table: readonly ScenarioRow[]): ScenarioRow[] {
  const classes = new Set(current.map((r) => r.class));
  const seen = new Set<number>();
  const matched: ScenarioRow[] = [];
  for (const row of table) {
    if (!classes.has(row.class) || seen.has(row.class)) continue;
    seen.add(row.class);
    matched.push(row);
  }
  const rest = current.filter((r) => !seen.has(r.class));
  return [...matched, ...rest].sort((a, b) => a.class - b.class);
}

/** Notices of a scenario table against the land cover, as AM5 shows them before merging. */
export interface TableNotices {
  /** A mode outside WALKING, BICYCLING, MOTORIZED (read as walking). */
  unknownMode: boolean;
  /** A class or label of the table absent from the land cover. */
  unmatched: boolean;
}

export function tableNotices(current: readonly ScenarioRow[], rows: Api.Rows | undefined): TableNotices {
  if (rows === undefined) return { unknownMode: false, unmatched: false };
  const m = column(rows, "mode");
  const unknownMode = m >= 0 && rows.rows.some((row) => !isMode(text(row[m]).toUpperCase()));
  const classes = new Set(current.map((r) => r.class));
  const labels = new Set(current.map((r) => r.label));
  const unmatched = tableRows(rows).some((r) => !classes.has(r.class) || !labels.has(r.label));
  return { unknownMode, unmatched };
}

/** Whether a class is set to 0 km/h (AM5 `srv_analysis_accessibility_tbl_speed_has_zero`). */
export const hasZero = (rows: readonly ScenarioRow[]) => rows.some((r) => r.speed === 0);

/** Edit of one cell; values come from the table as typed. */
export function editRow(rows: readonly ScenarioRow[], cls: number, field: "speed" | "mode", value: unknown): ScenarioRow[] {
  return rows.map((row) => {
    if (row.class !== cls) return row;
    if (field === "speed") {
      const speed = Number(text(value));
      return { ...row, speed: Number.isFinite(speed) && speed >= 0 ? speed : row.speed };
    }
    const mode = text(value).toUpperCase();
    return isMode(mode) ? { ...row, mode } : row;
  });
}

/** Job scenario: every class, 0 km/h ones as barriers, as AM5 keeps them (a replay shows every choice). */
export function toScenario(rows: readonly ScenarioRow[]): Jobs.ScenarioEntry[] {
  return rows.map((r) => ({ class: r.class, label: r.label, speed_kmh: r.speed, mode: r.mode.toLowerCase() as Jobs.TransportMode }));
}
