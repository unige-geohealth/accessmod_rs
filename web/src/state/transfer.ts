// Process state of an upload followed by a command (project.create,
// dataset.import): an explicit state machine, driven by the click handler
// that starts it, never by an effect.
import { useI18n } from "@accessmod/ui/i18n";
import { useCallback, useReducer } from "react";
import { api, issuesOf } from "../api/client";
import type { Api } from "../generated/types";

export type Transfer =
  | { state: "idle" }
  | { state: "uploading"; fraction: number }
  | { state: "processing" }
  | { state: "done"; label: string }
  | { state: "failed"; issues: string[] };

export type TransferMessage =
  | { type: "start" }
  | { type: "progress"; fraction: number }
  | { type: "uploaded" }
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
      return state.state === "uploading" ? { state: "processing" } : state;
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
 * `command` on the upload; its result names what was made.
 */
export function useTransfer() {
  const { t } = useI18n();
  const [state, dispatch] = useReducer(updateTransfer, IDLE);
  const start = useCallback(async (files: File[], command: (upload: Api.Upload) => Promise<string>) => {
    dispatch({ type: "start" });
    try {
      const upload = await api.upload(files, (fraction) => dispatch({ type: "progress", fraction }));
      dispatch({ type: "uploaded" });
      dispatch({ type: "done", label: await command(upload) });
    } catch (error) {
      dispatch({ type: "failed", issues: issuesOf(error, t) });
    }
  }, [t]);
  const reset = useCallback(() => dispatch({ type: "reset" }), []);
  return { state, start, reset };
}
