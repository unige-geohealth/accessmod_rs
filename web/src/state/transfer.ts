// Process state of an upload followed by a command (project.create,
// dataset.import): an explicit state machine, driven by the click handler
// that starts it, never by an effect.
import { useI18n } from "@accessmod/ui/i18n";
import { useCallback, useReducer } from "react";
import { api, issuesOf } from "../api/client";
import type { TaskRef } from "../api/tasks";
import type { Api } from "../generated/types";
import { jobPanel } from "./job-panel";

export type Transfer =
  | { state: "idle" }
  | { state: "uploading"; fraction: number }
  | { state: "processing"; task: TaskRef | null }
  | { state: "done" }
  | { state: "failed"; issues: string[] };

export type TransferMessage =
  | { type: "start" }
  | { type: "progress"; fraction: number }
  | { type: "uploaded" }
  | { type: "queued"; task: TaskRef }
  | { type: "done" }
  | { type: "failed"; issues: string[] }
  | { type: "reset" };

export const IDLE: Transfer = { state: "idle" };

export function updateTransfer(state: Transfer, message: TransferMessage): Transfer {
  switch (message.type) {
    case "start":
      return busy(state) ? state : { state: "uploading", fraction: 0 };
    case "progress":
      return state.state === "uploading" ? { state: "uploading", fraction: Math.min(1, Math.max(0, message.fraction)) } : state;
    case "uploaded":
      return state.state === "uploading" ? { state: "processing", task: null } : state;
    case "queued":
      return state.state === "processing" ? { state: "processing", task: message.task } : state;
    case "done":
      return busy(state) ? { state: "done" } : state;
    case "failed":
      // Once its job is queued, the job's end is notified, a failure too
      // (docs/ui.md, "Notifications"): nothing more to say here.
      if (state.state === "processing" && state.task !== null) return IDLE;
      return busy(state) ? { state: "failed", issues: message.issues } : state;
    case "reset":
      return busy(state) ? state : IDLE;
  }
}

/** Uploads started in this window, numbering each for the job panel. */
let uploads = 0;

export const busy = (t: Transfer) => t.state === "uploading" || t.state === "processing";

/**
 * A transfer and the function starting one: upload `files`, then run
 * `command` on the upload; it tells `queued` the task it queued (whose
 * end is notified). The job panel follows the upload, then that task.
 */
export function useTransfer() {
  const { t } = useI18n();
  const [state, dispatch] = useReducer(updateTransfer, IDLE);
  const start = useCallback(async (files: File[], command: (upload: Api.Upload, queued: (task: TaskRef) => void) => Promise<unknown>) => {
    dispatch({ type: "start" });
    // The upload and then its job show in the job panel too.
    const number = ++uploads;
    jobPanel.dispatch({ type: "upload", upload: number });
    try {
      const upload = await api.upload(files, (fraction) => {
        dispatch({ type: "progress", fraction });
        jobPanel.dispatch({ type: "progress", upload: number, fraction });
      });
      dispatch({ type: "uploaded" });
      await command(upload, (task) => {
        dispatch({ type: "queued", task });
        jobPanel.dispatch({ type: "follow", task, upload: number });
      });
      dispatch({ type: "done" });
    } catch (error) {
      // Refused before any job: the form says why, the panel closes.
      jobPanel.dispatch({ type: "drop", upload: number });
      dispatch({ type: "failed", issues: issuesOf(error, t) });
    }
  }, [t]);
  // A command on no upload (an export, an archive): straight to processing.
  const run = useCallback(async (command: (queued: (task: TaskRef) => void) => Promise<unknown>) => {
    dispatch({ type: "start" });
    dispatch({ type: "uploaded" });
    try {
      await command((task) => {
        dispatch({ type: "queued", task });
        jobPanel.dispatch({ type: "follow", task });
      });
      dispatch({ type: "done" });
    } catch (error) {
      dispatch({ type: "failed", issues: issuesOf(error, t) });
    }
  }, [t]);
  const reset = useCallback(() => dispatch({ type: "reset" }), []);
  return { state, start, run, reset };
}
