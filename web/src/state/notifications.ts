// Notifications: a job (a run, an operation making a project) that ended,
// and a job just queued from an analysis tab. Events (api/events.ts) and
// Compute dispatch them; the toaster shows them. A success, a queued job
// or a cancellation fades after a while (a selector on the clock), a
// failure stays until closed.
import type { Api } from "../generated/types";
import { createStore } from "./store";

export interface Notice {
  /** Job and its end: an event repeated is one notice. */
  id: string;
  project: string;
  /** Run id, or operation id when `pending`. */
  run: number;
  pending: boolean;
  /** Analysis or operation name. */
  analysis: string;
  kind: Api.RunKind;
  status: Api.RunStatus;
  /** Outcome, or the first issue. */
  text: string;
  /** Seconds since the epoch. */
  at: number;
}

export interface Notifications {
  notices: readonly Notice[];
  /** Ids of the notices closed (or faded and seen): never shown again. */
  closed: readonly string[];
}

export type NotificationMessage = { type: "notice"; notice: Notice } | { type: "dismiss"; id: string };

export const NO_NOTIFICATIONS: Notifications = { notices: [], closed: [] };

/** Notices kept at most. */
const KEPT = 20;
/** Seconds a success or a cancellation stays. */
export const FADE_S = 8;

const ended = (status: Api.RunStatus) => status !== "queued" && status !== "running";

export function updateNotifications(state: Notifications, message: NotificationMessage): Notifications {
  switch (message.type) {
    case "notice": {
      // A job ended, or was just queued: never one still running.
      if (message.notice.status === "running" || state.notices.some((n) => n.id === message.notice.id)) return state;
      const notices = [...state.notices, message.notice];
      // Beyond the bound, the oldest that is closed or not a failure goes
      // first: a failure stays until closed.
      while (notices.length > KEPT) {
        const at = notices.findIndex((n) => state.closed.includes(n.id) || !(n.status === "failed" || n.status === "interrupted"));
        notices.splice(at < 0 ? 0 : at, 1);
      }
      const ids = new Set(notices.map((n) => n.id));
      return { notices, closed: state.closed.filter((id) => ids.has(id)) };
    }
    case "dismiss":
      return state.closed.includes(message.id) || !state.notices.some((n) => n.id === message.id)
        ? state
        : { ...state, closed: [...state.closed, message.id] };
  }
}

/** The notice of a run event, when it ended. */
export function runNotice(project: string, run: Api.RunRecord, at: number): Notice | null {
  if (!ended(run.status)) return null;
  return {
    id: `${project}/${run.id}/${run.status}`,
    project,
    run: run.id,
    pending: false,
    analysis: run.analysis,
    kind: run.kind ?? "analysis",
    status: run.status,
    text: run.outcome ?? run.error?.issues[0] ?? "",
    at,
  };
}

/** The notice of analysis `run`, just queued from a tab: it opens the job. */
export function queuedNotice(project: string, run: Api.RunRecord, at: number): Notice {
  return {
    id: `${project}/${run.id}/queued`,
    project,
    run: run.id,
    pending: false,
    analysis: run.analysis,
    kind: "analysis",
    status: "queued",
    text: run.tags.join(" "),
    at,
  };
}

/** The notice of an operation making a project, when it ended. */
export function operationNotice(operation: Api.PendingOperation, at: number): Notice | null {
  if (!ended(operation.status)) return null;
  return {
    id: `:pending/${operation.id}/${operation.status}`,
    project: operation.project,
    run: operation.id,
    pending: true,
    analysis: operation.operation,
    kind: "manage",
    status: operation.status,
    text: operation.error?.issues[0] ?? operation.name,
    at,
  };
}

/** Notices shown at `now`: not closed; failures always, others for a while. */
export function visibleNotices(state: Notifications, now: number): Notice[] {
  return state.notices.filter(
    (n) => !state.closed.includes(n.id) && (n.status === "failed" || n.status === "interrupted" || now - n.at < FADE_S),
  );
}

export const notifications = createStore(NO_NOTIFICATIONS, updateNotifications);
