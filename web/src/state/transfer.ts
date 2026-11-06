// Process state of an upload followed by a command (project.create,
// dataset.import): an explicit state machine, driven by the click handler
// that starts it, never by an effect.
import { useI18n } from "@accessmod/ui/i18n";
import { useCallback, useReducer } from "react";
import { api, issuesOf } from "../api/client";
import type { TaskRef } from "../api/tasks";
import type { Api } from "../generated/types";

export type Transfer =
  | { state: "idle" }
  | { state: "uploading"; fraction: number }
  | { state: "processing"; task: TaskRef | null }
  | { state: "done"; label: string }
  | { state: "failed"; issues: string[] };

export type TransferMessage =
  | { type: "start" }
  | { type: "progress"; fraction: number }
  | { type: "uploaded" }
  | { type: "queued"; task: TaskRef }
  | { type: "done"; label: string }
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
      return busy(state) ? { state: "done", label: message.label } : state;
    case "failed":
      return busy(state) ? { state: "failed", issues: message.issues } : state;
    case "reset":
      return busy(state) ? state : IDLE;
  }
}

export const busy = (t: Transfer) => t.state === "uploading" || t.state === "processing";

/**
 * A transfer and the function starting one: upload `files`, then run
 * `command` on the upload; it tells `queued` the task it queued, and its
 * result names what was made.
 */
export function useTransfer() {
  const { t } = useI18n();
  const [state, dispatch] = useReducer(updateTransfer, IDLE);
  const start = useCallback(async (files: File[], command: (upload: Api.Upload, queued: (task: TaskRef) => void) => Promise<string>) => {
    dispatch({ type: "start" });
    try {
      const upload = await api.upload(files, (fraction) => dispatch({ type: "progress", fraction }));
      dispatch({ type: "uploaded" });
      dispatch({ type: "done", label: await command(upload, (task) => dispatch({ type: "queued", task })) });
    } catch (error) {
      dispatch({ type: "failed", issues: issuesOf(error, t) });
    }
  }, [t]);
  // A command on no upload (an export, an archive): straight to processing.
  const run = useCallback(async (command: (queued: (task: TaskRef) => void) => Promise<string>) => {
    dispatch({ type: "start" });
    dispatch({ type: "uploaded" });
    try {
      dispatch({ type: "done", label: await command((task) => dispatch({ type: "queued", task })) });
    } catch (error) {
      dispatch({ type: "failed", issues: issuesOf(error, t) });
    }
  }, [t]);
  const reset = useCallback(() => dispatch({ type: "reset" }), []);
  return { state, start, run, reset };
}
