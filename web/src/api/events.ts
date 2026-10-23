// Server-sent events: the server pushes run states, progress and catalogue
// changes; the client refreshes the matching queries and feeds the runs
// store. This is the only place events enter the client.
import { useEffect } from "react";
import type { Api } from "../generated/types";
import { NO_RUNS, updateRuns } from "../state/runs";
import { createStore } from "../state/store";
import { keys, queryClient } from "./queries";

/** What events say about runs, by project. */
export const runsStore = createStore(NO_RUNS, updateRuns);

const NAMES = ["run", "progress", "catalogue", "projects"] as const;

/** The server dropped events for this client: reload everything shown. */
function resync() {
  void queryClient.invalidateQueries();
}

function handle(event: Api.Event) {
  runsStore.dispatch({ type: "event", event });
  switch (event.type) {
    case "run":
      if (event.run.status !== "queued" && event.run.status !== "running") {
        void queryClient.invalidateQueries({ queryKey: keys.datasets(event.project) });
      }
      void queryClient.invalidateQueries({ queryKey: keys.runs(event.project) });
      return;
    case "catalogue":
      void queryClient.invalidateQueries({ queryKey: keys.datasets(event.project) });
      void queryClient.invalidateQueries({ queryKey: keys.jobs(event.project) });
      return;
    case "projects":
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      return;
    case "progress":
      return;
  }
}

/** Follow the events of `project` while mounted: a subscription only. */
export function useEvents(project: string | null) {
  useEffect(() => {
    const url =
      project === null ? "/api/events" : `/api/events?project=${encodeURIComponent(project)}`;
    const source = new EventSource(url);
    const listener = (e: MessageEvent<string>) => {
      try {
        handle(JSON.parse(e.data) as Api.Event);
      } catch {
        // A malformed event is ignored; the next query refresh catches up.
      }
    };
    for (const name of NAMES) source.addEventListener(name, listener);
    source.addEventListener("resync", resync);
    // After a reconnection, events may have been missed: reload.
    source.onopen = resync;
    return () => source.close();
  }, [project]);
}
