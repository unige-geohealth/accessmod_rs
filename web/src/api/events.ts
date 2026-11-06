// Server-sent events: the server pushes run states, progress and catalogue
// changes; the client refreshes the matching queries. This is the only
// place events enter the client.
import { useEffect } from "react";
import type { Api } from "../generated/types";
import { keys, type ProgressByRun, queryClient } from "./queries";
import { CLIENT } from "./client";
import { notifications, operationNotice, runNotice } from "../state/notifications";
import { settleCheck, settleOperation, settleRun } from "./tasks";

const now = () => Date.now() / 1000;

/** Insert or replace `run` in the cached runs of `project`. */
function upsertRun(project: string, run: Api.RunRecord) {
  queryClient.setQueryData<Api.RunRecord[]>(keys.runs(project), (runs) => {
    if (runs === undefined) return undefined;
    const at = runs.findIndex((r) => r.id === run.id);
    return at < 0 ? [...runs, run] : runs.map((r, i) => (i === at ? run : r));
  });
}

const NAMES = ["run", "runs", "progress", "catalogue", "projects", "queue", "pending", "check"] as const;

/** The server dropped events for this client: reload everything shown. */
function resync() {
  void queryClient.invalidateQueries();
}

function handle(event: Api.Event) {
  switch (event.type) {
    case "run":
      upsertRun(event.project, event.run);
      settleRun(event.project, event.run);
      {
        const notice = runNotice(event.project, event.run, now());
        if (notice !== null) notifications.dispatch({ type: "notice", notice });
      }
      void queryClient.invalidateQueries({ queryKey: keys.allRuns });
      if (event.run.status !== "queued" && event.run.status !== "running") {
        // A finished run registers its outputs as datasets.
        void queryClient.invalidateQueries({ queryKey: keys.datasets(event.project) });
        void queryClient.invalidateQueries({ queryKey: keys.runs(event.project) });
        if (event.run.kind === "manage" && event.run.analysis === "archive") {
          void queryClient.invalidateQueries({ queryKey: keys.archives(event.project) });
        }
      }
      return;
    case "runs":
      void queryClient.invalidateQueries({ queryKey: keys.runs(event.project) });
      void queryClient.invalidateQueries({ queryKey: keys.allRuns });
      return;
    case "queue":
      queryClient.setQueryData<Api.QueueEntry[]>(keys.queue, (all) => [
        ...(all ?? []).filter((e) => e.lane !== event.lane),
        ...event.entries,
      ]);
      return;
    case "pending":
      settleOperation(event.operation);
      {
        const notice = operationNotice(event.operation, now());
        if (notice !== null) notifications.dispatch({ type: "notice", notice });
      }
      queryClient.setQueryData<Api.PendingOperation[]>(keys.operations, (all) => [
        ...(all ?? []).filter((o) => o.id !== event.operation.id),
        event.operation,
      ]);
      // A done one is a run of the project it made.
      if (event.operation.status === "done") {
        void queryClient.invalidateQueries({ queryKey: keys.projects });
        void queryClient.invalidateQueries({ queryKey: keys.allRuns });
      }
      return;
    case "check":
      // Pre-analyses of other tabs are theirs.
      if (event.check.client !== CLIENT) return;
      settleCheck(event.check);
      queryClient.setQueryData<Api.Check | null>(keys.check(event.check.project, event.check.analysis), (current) =>
        current === undefined || current === null || current.id <= event.check.id ? event.check : current,
      );
      return;
    case "catalogue":
      void queryClient.invalidateQueries({ queryKey: keys.datasets(event.project) });
      return;
    case "projects":
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
      return;
    case "progress":
      queryClient.setQueryData<ProgressByRun>(keys.progress(event.project), (all) => ({ ...all, [event.run]: event.progress }));
      return;
  }
}

/** Follow the events of `project` while mounted: a subscription only. */
export function useEvents(project: string | null) {
  useEffect(() => {
    const url = project === null ? "/api/events" : `/api/events?project=${encodeURIComponent(project)}`;
    const listener = (e: MessageEvent<string>) => {
      try {
        handle(JSON.parse(e.data) as Api.Event);
      } catch {
        // A malformed event is ignored; the next query refresh catches up.
      }
    };
    let source: EventSource;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const connect = () => {
      source = new EventSource(url);
      for (const name of NAMES) source.addEventListener(name, listener);
      source.addEventListener("resync", resync);
      // After a reconnection, events may have been missed: reload.
      source.onopen = resync;
      // EventSource retries a dropped stream itself, but gives up on an
      // error status (a proxy answering while the server restarts).
      source.onerror = () => {
        if (source.readyState === EventSource.CLOSED) retry = setTimeout(connect, 2000);
      };
    };
    connect();
    return () => {
      clearTimeout(retry);
      source.close();
    };
  }, [project]);
}
