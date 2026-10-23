// Runs of the open project, mirrored from server events: a state machine
// per run. The server owns runs; this store only follows its events.
import type { Api } from "../generated/types";

export type RunProcess =
  | { status: "queued"; run: Api.RunRecord }
  | { status: "running"; run: Api.RunRecord; progress: Api.Progress | null }
  | { status: "done" | "failed" | "cancelled" | "interrupted"; run: Api.RunRecord };

export interface RunsState {
  project: string | null;
  runs: Record<number, RunProcess>;
}

export type RunsMessage =
  | { type: "open"; project: string | null }
  | { type: "loaded"; project: string; runs: Api.RunRecord[] }
  | { type: "event"; event: Api.Event };

export const NO_RUNS: RunsState = { project: null, runs: {} };

function fromRecord(run: Api.RunRecord): RunProcess {
  switch (run.status) {
    case "queued":
      return { status: "queued", run };
    case "running":
      return { status: "running", run, progress: null };
    case "done":
    case "failed":
    case "cancelled":
    case "interrupted":
      return { status: run.status, run };
  }
}

export function updateRuns(state: RunsState, message: RunsMessage): RunsState {
  switch (message.type) {
    case "open":
      return message.project === state.project ? state : { project: message.project, runs: {} };
    case "loaded": {
      if (message.project !== state.project) return state;
      const runs: Record<number, RunProcess> = {};
      for (const run of message.runs) {
        // A run already followed through events keeps its live progress.
        const known = state.runs[run.id];
        runs[run.id] =
          known?.status === "running" && run.status === "running" ? known : fromRecord(run);
      }
      return { ...state, runs };
    }
    case "event": {
      const event = message.event;
      switch (event.type) {
        case "run":
          if (event.project !== state.project) return state;
          return { ...state, runs: { ...state.runs, [event.run.id]: fromRecord(event.run) } };
        case "progress": {
          if (event.project !== state.project) return state;
          const known = state.runs[event.run];
          // Progress implies the run is running; its record comes with it.
          if (known === undefined) return state;
          if (known.status !== "queued" && known.status !== "running") return state;
          return {
            ...state,
            runs: {
              ...state.runs,
              [event.run]: {
                status: "running",
                run: { ...known.run, status: "running" },
                progress: event.progress,
              },
            },
          };
        }
        case "catalogue":
        case "projects":
          return state;
      }
    }
  }
}

/** Fraction done of a running run's current stage, when known. */
export function fraction(process: RunProcess): number | null {
  if (process.status !== "running" || process.progress === null) return null;
  const { done, total } = process.progress;
  return total === undefined || total === null || total === 0 ? null : Math.min(1, done / total);
}
