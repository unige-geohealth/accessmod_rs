// What server events say about runs, by project: the latest record of each
// run and the latest progress of running ones. The server owns runs; views
// combine this with the listed records (derived, never stored).
import type { Api } from "../generated/types";

export interface RunsState {
  /** Latest record of each run seen in events: project -> id -> record. */
  runs: Readonly<Record<string, Readonly<Record<number, Api.RunRecord>>>>;
  /** Latest progress of running runs: project -> id -> progress. */
  progress: Readonly<Record<string, Readonly<Record<number, Api.Progress>>>>;
}

export type RunsMessage = { type: "event"; event: Api.Event };

export const NO_RUNS: RunsState = { runs: {}, progress: {} };

const ACTIVE = new Set<Api.RunStatus>(["queued", "running"]);

export function updateRuns(state: RunsState, message: RunsMessage): RunsState {
  const event = message.event;
  switch (event.type) {
    case "run": {
      const { project, run } = event;
      const runs = { ...state.runs, [project]: { ...state.runs[project], [run.id]: run } };
      if (ACTIVE.has(run.status)) return { ...state, runs };
      // A finished run has no progress any more.
      const { [run.id]: _, ...rest } = state.progress[project] ?? {};
      return { runs, progress: { ...state.progress, [project]: rest } };
    }
    case "progress": {
      const { project, run, progress } = event;
      const known = state.runs[project]?.[run];
      if (known !== undefined && !ACTIVE.has(known.status)) return state;
      return {
        ...state,
        progress: { ...state.progress, [project]: { ...state.progress[project], [run]: progress } },
      };
    }
    case "catalogue":
    case "projects":
      return state;
  }
}

/** A run as shown: the listed record unless events know a newer state of an
 * active run, with its progress while it runs. */
export function showRun(
  record: Api.RunRecord,
  live: Readonly<Record<number, Api.RunRecord>> | undefined,
  progress: Readonly<Record<number, Api.Progress>> | undefined,
): { run: Api.RunRecord; progress: Api.Progress | null } {
  const run = ACTIVE.has(record.status) ? (live?.[record.id] ?? record) : record;
  const current = progress?.[record.id];
  if (!ACTIVE.has(run.status) || current === undefined) return { run, progress: null };
  // Progress means running, even before the running event arrives.
  return { run: { ...run, status: "running" }, progress: current };
}

/** Fraction done of a stage, when its total is known. */
export function fraction(progress: Api.Progress | null): number | null {
  if (progress === null) return null;
  const { done, total } = progress;
  return total === undefined || total === null || total === 0 ? null : Math.min(1, done / total);
}
