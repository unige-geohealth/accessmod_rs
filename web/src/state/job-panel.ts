// The job panel: the job the user just started in this window, followed
// in a modal, as AM5 does (docs/ui.md, "Notifications"). Unlike AM5, it
// can be closed: the job goes on, in Jobs. What the panel shows of the
// job (status, progress, end) is server state, selected from the caches
// the events keep; this store holds only what the panel follows.
import type { TaskRef } from "../api/tasks";
import type { Notice } from "./notifications";
import { createStore } from "./store";

/** Closed, an upload (`upload` numbers it), or a job. */
export type JobPanel =
  | { kind: "closed" }
  | { kind: "upload"; upload: number; fraction: number }
  | { kind: "job"; task: TaskRef };

export type JobPanelMessage =
  | { type: "upload"; upload: number }
  | { type: "progress"; upload: number; fraction: number }
  /** A job queued, after upload `upload` when it followed one. */
  | { type: "follow"; task: TaskRef; upload?: number }
  /** Upload `upload`, or its command, failed before any job. */
  | { type: "drop"; upload: number }
  | { type: "close" };

export const CLOSED: JobPanel = { kind: "closed" };

export function updateJobPanel(state: JobPanel, message: JobPanelMessage): JobPanel {
  switch (message.type) {
    case "upload":
      return { kind: "upload", upload: message.upload, fraction: 0 };
    case "progress":
      return state.kind === "upload" && state.upload === message.upload
        ? { ...state, fraction: Math.min(1, Math.max(0, message.fraction)) }
        : state;
    case "follow":
      // The job of an upload only while the panel still shows that upload:
      // closed meanwhile, or taken by another, the job goes on in Jobs.
      if (message.upload !== undefined && !(state.kind === "upload" && state.upload === message.upload)) return state;
      // A new job takes the panel over; the one before stays in Jobs.
      return { kind: "job", task: message.task };
    case "drop":
      return state.kind === "upload" && state.upload === message.upload ? CLOSED : state;
    case "close":
      return CLOSED;
  }
}

export const jobPanel = createStore(CLOSED, updateJobPanel);

/** Whether `notice` is about `task`. */
export const isNoticeOf = (notice: Pick<Notice, "pending" | "project" | "run">, task: TaskRef) =>
  task.kind === "operation"
    ? notice.pending && notice.run === task.id
    : !notice.pending && notice.project === task.project && notice.run === task.id;

/** The task the panel shows, if any. */
export const followed = (state: JobPanel): TaskRef | null => (state.kind === "job" ? state.task : null);
