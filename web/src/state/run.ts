// A run started from an analysis screen, seen through the server's run
// records (refreshed by events) and progress events. Pure selectors; the
// screen keeps only the id of the run it started and whether its outcome
// was acknowledged.
import type { Api } from "../generated/types";

export type RunView =
  | { state: "idle" }
  | { state: "queued" }
  | { state: "running"; progress: Api.Progress | undefined }
  | { state: "done"; record: Api.RunRecord }
  | { state: "failed"; record: Api.RunRecord };

/** What the screen shows of run `id`. */
export function runView(runs: readonly Api.RunRecord[] | undefined, progress: Readonly<Record<number, Api.Progress>> | undefined, id: number | null): RunView {
  if (id === null) return { state: "idle" };
  const record = runs?.find((r) => r.id === id);
  switch (record?.status) {
    case undefined:
    case "queued":
      return { state: "queued" };
    case "running":
      return { state: "running", progress: progress?.[id] };
    case "done":
      return { state: "done", record };
    case "failed":
    case "cancelled":
    case "interrupted":
      return { state: "failed", record };
  }
}

export const busy = (view: RunView) => view.state === "queued" || view.state === "running";

/** Percent of a stage, when its total is known. */
export function percent(progress: Api.Progress | undefined): number | undefined {
  if (!progress?.total) return undefined;
  return Math.min(100, Math.round((progress.done / progress.total) * 100));
}

/** Duration of a finished run, minutes (presentation only). */
export function minutes(record: Api.RunRecord): number {
  return Math.max(0, (record.finished ?? record.started) - record.started) / 60;
}

/** Finished analyses of the project, newest first: those whose job, inputs and outputs Data shows (operations are not jobs). */
export function doneRuns(runs: readonly Api.RunRecord[] | undefined): Api.RunRecord[] {
  return (runs ?? []).filter((r) => r.status === "done" && (r.kind ?? "analysis") === "analysis").sort((a, b) => b.id - a.id);
}

/** The latest finished run ("Filter last results"), if any. */
export const lastRun = (runs: readonly Api.RunRecord[] | undefined): number | null => doneRuns(runs)[0]?.id ?? null;
