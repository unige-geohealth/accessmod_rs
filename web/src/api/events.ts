// Server-sent events: the server pushes run states, progress and catalogue
// changes; the client refreshes the matching queries. This is the only
// place events enter the client.
import { useEffect } from "react";
import type { Api } from "../generated/types";
import { keys, type ProgressByRun, queryClient } from "./queries";

/** Insert or replace `run` in the cached runs of `project`. */
function upsertRun(project: string, run: Api.RunRecord) {
  queryClient.setQueryData<Api.RunRecord[]>(keys.runs(project), (runs) => {
    if (runs === undefined) return undefined;
    const at = runs.findIndex((r) => r.id === run.id);
    return at < 0 ? [...runs, run] : runs.map((r, i) => (i === at ? run : r));
  });
}

const NAMES = ["run", "progress", "catalogue", "projects"] as const;

/** The server dropped events for this client: reload everything shown. */
function resync() {
  void queryClient.invalidateQueries();
}

function handle(event: Api.Event) {
  switch (event.type) {
    case "run":
      upsertRun(event.project, event.run);
      if (event.run.status !== "queued" && event.run.status !== "running") {
        // A finished run registers its outputs as datasets.
        void queryClient.invalidateQueries({ queryKey: keys.datasets(event.project) });
        void queryClient.invalidateQueries({ queryKey: keys.runs(event.project) });
      }
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
