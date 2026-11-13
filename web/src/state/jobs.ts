// Jobs (in place of AM5 Logs): the runs of every project, one queue for
// all. Selectors only: rows of the table, the summary line, what the
// details show. Runs and progress are server state.
import type { Key } from "@accessmod/ui/i18n";
import type { Api } from "../generated/types";
import { percent, stageLabel } from "./run";

/** Interface name of an analysis (AM5 tab or tool labels). */
const ANALYSES: Record<string, Key> = {
  "travel-time": "analysis_accessibility_main_tab",
  "geographic-coverage": "analysis_coverage_main_tab",
  referral: "analysis_referral_main_tab",
  "zonal-statistics": "analysis_zonal_stats_main_tab",
  "scaling-up": "analysis_scalingup_main_tab",
  optimization: "analysis_optimization_main_tab",
  "merge-land-cover": "tools_merge_lc_btn",
  "adjust-population-distribution": "tools_correct_pop_btn",
  // Operations (runs of kind `manage`).
  archive: "jobs.op_archive",
  "project-export": "jobs.op_project_export",
  "dataset-import": "jobs.op_dataset_import",
  "project-create": "jobs.op_project_create",
  "project-import": "jobs.op_project_import",
  "demo-reset": "jobs.op_demo_reset",
};

export const analysisLabel = (t: (key: Key) => string, name: string): string => (ANALYSES[name] ? t(ANALYSES[name]) : name);

/**
 * A job of the table: a run of a project, or an operation making a
 * project (`pending`: no project holds it yet), shown as a run.
 */
export type JobItem = Api.JobEntry & { pending?: true };

/** Row key of a job: runs are numbered per project, operations apart. */
export const jobKey = (entry: Pick<JobItem, "project" | "run" | "pending">) =>
  entry.pending ? `:pending/${entry.run.id}` : `${entry.project}/${entry.run.id}`;

/** What the table lists: everything, analyses or operations. */
export type JobKind = "all" | "analysis" | "manage";

export const jobKind = (value: string | undefined): JobKind => (value === "analysis" || value === "manage" ? value : "all");

/** Jobs of `kind`. */
export const ofKind = (entries: readonly JobItem[], kind: JobKind) =>
  kind === "all" ? entries : entries.filter((e) => (e.run.kind ?? "analysis") === kind);

/**
 * Operations making projects, as jobs: those not done (a done one is a run
 * of the project it made), newest first.
 */
export function pendingJobs(operations: readonly Api.PendingOperation[]): JobItem[] {
  return operations
    .filter((o) => o.status !== "done")
    .map(
      (o): JobItem => ({
        pending: true,
        project: o.project,
        name: o.name,
        run: {
          id: o.id,
          analysis: o.operation,
          kind: "manage",
          tags: [],
          job: {},
          status: o.status,
          output_dir: "",
          outcome: null,
          error: o.error ?? null,
          started: o.started,
          finished: o.finished ?? null,
          datasets: [],
          inputs: [],
          overwrite: false,
        },
      }),
    )
    .sort((a, b) => b.run.started - a.run.started);
}

/** Jobs newest first (by start), pending operations among the runs. */
export const newestFirst = (entries: readonly JobItem[]): JobItem[] => [...entries].sort((a, b) => b.run.started - a.run.started);

/** Place of each queued or running job in its lane, by row key. */
export function queuePlaces(queue: readonly Api.QueueEntry[]): Readonly<Record<string, Api.QueueEntry>> {
  return Object.fromEntries(queue.map((e) => [e.project === ":pending" ? `:pending/${e.run}` : `${e.project}/${e.run}`, e] as const));
}

export const isActive = (run: Api.RunRecord) => run.status === "queued" || run.status === "running";

/** Seconds a run took, or has taken so far (`now`, seconds since the epoch). */
export const seconds = (run: Api.RunRecord, now: number) => Math.max(0, (run.finished ?? (isActive(run) ? now : run.started)) - run.started);

/** "1 h 02 min", "3 min 05 s", "12 s" (presentation only). */
export function duration(total: number): string {
  const s = Math.round(total);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  if (h > 0) return `${h} h ${pad(m)} min`;
  if (m > 0) return `${m} min ${pad(s % 60)} s`;
  return `${s % 60} s`;
}

export interface Summary {
  total: number;
  running: number;
  queued: number;
  failed: number;
  /** Seconds of the finished runs. */
  computing: number;
}

export function summary(entries: readonly Api.JobEntry[]): Summary {
  const count = (status: Api.RunStatus) => entries.filter((e) => e.run.status === status).length;
  const computing = entries.reduce((sum, e) => sum + (e.run.finished === null || e.run.finished === undefined ? 0 : e.run.finished - e.run.started), 0);
  return { total: entries.length, running: count("running"), queued: count("queued"), failed: count("failed") + count("interrupted"), computing };
}

export type JobRow = Record<string, string | number | null> & { key: string };

/** Rows of the table, as listed (newest first); a running job shows its stage and percent. */
export function jobRows(
  entries: readonly JobItem[],
  t: (key: Key) => string,
  progress: Readonly<Record<string, Api.Progress>>,
  now: number,
  format: (epochSeconds: number) => string,
  places: Readonly<Record<string, Api.QueueEntry>> = {},
): JobRow[] {
  return entries.map((e) => {
    const key = jobKey(e);
    const stage = progress[key];
    const value = percent(stage);
    const place = places[key];
    const base = t(`jobs.status_${e.run.status}` as Key);
    const status = place?.paused
      ? `${base} (${t("task.paused")})`
      : e.run.status === "queued" && place !== undefined && place.position > 0
        ? `${base} (#${place.position})`
        : base;
    return {
      key,
      project: e.name,
      analysis: analysisLabel(t, e.run.analysis),
      tags: e.run.tags.join(" "),
      status: e.run.status === "running" && stage ? `${status}: ${stageLabel(t, stage.stage)}${value === undefined ? "" : ` ${value} %`}` : status,
      started: format(e.run.started),
      duration: duration(seconds(e.run, now)),
      outcome: e.run.outcome ?? e.run.error?.issues[0] ?? "",
    };
  });
}
