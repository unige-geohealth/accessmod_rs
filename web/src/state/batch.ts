// Process state of commands run one after the other on several items
// (rename or delete datasets, delete a project), with the outcome of each.
// Started by a click handler, never by an effect.
import { useI18n } from "@accessmod/ui/i18n";
import { useCallback, useReducer } from "react";
import { issuesOf } from "../api/client";

export interface Outcome {
  label: string;
  /** `null` when the command succeeded. */
  issues: string[] | null;
}

export type Batch =
  | { state: "idle" }
  | { state: "running"; done: number; total: number }
  | { state: "done"; outcomes: Outcome[] };

export type BatchMessage =
  | { type: "start"; total: number }
  | { type: "step"; outcome: Outcome }
  | { type: "reset" };

export const IDLE: Batch = { state: "idle" };

/** Outcomes so far, kept aside while running so `done` can list them all. */
interface Run {
  batch: Batch;
  outcomes: Outcome[];
}

export function updateBatch(run: Run, message: BatchMessage): Run {
  switch (message.type) {
    case "start":
      if (run.batch.state === "running") return run;
      return message.total === 0
        ? { batch: { state: "done", outcomes: [] }, outcomes: [] }
        : { batch: { state: "running", done: 0, total: message.total }, outcomes: [] };
    case "step": {
      if (run.batch.state !== "running") return run;
      const outcomes = [...run.outcomes, message.outcome];
      const { total } = run.batch;
      return {
        batch: outcomes.length >= total ? { state: "done", outcomes } : { state: "running", done: outcomes.length, total },
        outcomes,
      };
    }
    case "reset":
      return run.batch.state === "running" ? run : { batch: IDLE, outcomes: [] };
  }
}

export const INITIAL: Run = { batch: IDLE, outcomes: [] };

/** A batch and the function running one: `step` per item, in order. */
export function useBatch() {
  const { t } = useI18n();
  const [run, dispatch] = useReducer(updateBatch, INITIAL);
  const start = useCallback(async <T,>(items: readonly T[], label: (item: T) => string, step: (item: T) => Promise<unknown>) => {
    dispatch({ type: "start", total: items.length });
    const succeeded: T[] = [];
    for (const item of items) {
      try {
        await step(item);
        succeeded.push(item);
        dispatch({ type: "step", outcome: { label: label(item), issues: null } });
      } catch (error) {
        dispatch({ type: "step", outcome: { label: label(item), issues: issuesOf(error, t) } });
      }
    }
    return succeeded;
  }, [t]);
  const reset = useCallback(() => dispatch({ type: "reset" }), []);
  return { batch: run.batch, start, reset };
}
