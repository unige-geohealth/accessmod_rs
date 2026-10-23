// A job being edited: the document `run` will send, exactly as the CLI
// would replay it (paths relative to the project). Changed only through
// messages; `update` is pure.

import { type Json, type Path, removeIn, setIn } from "../forms/schema";
import type { Api } from "../generated/types";

export interface JobDraft {
  analysis: string;
  name: string;
  document: Json;
  /** The saved job it edits, at the revision read. */
  saved: { id: number; revision: number } | null;
  /** Changed since it was loaded or saved. */
  dirty: boolean;
}

export type JobMessage =
  | { type: "new"; analysis: string; document: Json }
  | { type: "load"; job: Api.SavedJob }
  | { type: "saved"; job: Api.SavedJob }
  /** A save of an older state of this draft succeeded: only the saved job
   * and its revision change; the current edits stay. */
  | { type: "revised"; id: number; revision: number }
  | { type: "set"; path: Path; value: Json | undefined }
  | { type: "remove"; path: Path }
  | { type: "append"; path: Path; value: Json }
  | { type: "rename"; name: string };

export const EMPTY_JOB: JobDraft = {
  analysis: "",
  name: "",
  document: {},
  saved: null,
  dirty: false,
};

export function updateJob(state: JobDraft, message: JobMessage): JobDraft {
  switch (message.type) {
    case "new":
      return {
        analysis: message.analysis,
        name: "",
        document: message.document,
        saved: null,
        dirty: false,
      };
    case "load":
    case "saved":
      return {
        analysis: message.job.analysis,
        name: message.job.name,
        document: message.job.document as Json,
        saved: { id: message.job.id, revision: message.job.revision },
        dirty: false,
      };
    case "revised":
      return { ...state, saved: { id: message.id, revision: message.revision } };
    case "set":
      return {
        ...state,
        document: setIn(state.document, message.path, message.value),
        dirty: true,
      };
    case "remove":
      return { ...state, document: removeIn(state.document, message.path), dirty: true };
    case "append": {
      const list = message.path.reduce<Json | undefined>(
        (v, k) =>
          v === undefined || v === null
            ? undefined
            : typeof k === "number"
              ? Array.isArray(v)
                ? v[k]
                : undefined
              : !Array.isArray(v) && typeof v === "object"
                ? v[k]
                : undefined,
        state.document,
      );
      const next = [...(Array.isArray(list) ? list : []), message.value];
      return { ...state, document: setIn(state.document, message.path, next), dirty: true };
    }
    case "rename":
      return message.name === state.name ? state : { ...state, name: message.name, dirty: true };
  }
}
