// Jobs (in place of AM5 Logs): the task manager. The analyses and the
// operations (imports, exports, archives, project creation) of every
// project, newest first, with their progress, place in the queue and
// pause; a summary line; the details of the selected job: issues, inputs,
// outputs, Pause / Resume, Cancel, Retry, Export job, Remove. The
// selection, the scope and the kind are in the URL.
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { RadioField } from "@accessmod/ui/am/form";
import { Callout } from "@accessmod/ui/am/messages";
import { CenterTitle, SectionCard } from "@accessmod/ui/am/section";
import { T, useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import { useAllRuns, useOperations, useProgressOf, useQueue } from "../api/queries";
import { navigate, type Route } from "../app/route";
import { duration, isActive, jobKey, jobKind, jobRows, newestFirst, ofKind, pendingJobs, queuePlaces, summary } from "../state/jobs";
import { useFmt, useNow } from "./common";
import { JobDetails } from "./job-details";

export function JobsScreen({ route }: { route: Route }) {
  const { t, lang } = useI18n();
  const f = useFmt();
  const all = useAllRuns();
  const operations = useOperations().data;
  const queue = useQueue().data;
  const scope = route.query.scope === "project" && route.project !== null ? "project" : "all";
  const kind = jobKind(route.query.kind);
  const entries = useMemo(
    () =>
      ofKind(
        newestFirst([...pendingJobs(operations ?? []), ...(all.data ?? [])]).filter((e) => scope === "all" || (!e.pending && e.project === route.project)),
        kind,
      ),
    [all.data, operations, scope, kind, route.project],
  );
  const places = useMemo(() => queuePlaces(queue ?? []), [queue]);
  // A run per lane may be running: an analysis, an operation.
  const running = entries.filter((e) => e.run.status === "running");
  const progress = useProgressOf([...new Set(running.filter((e) => !e.pending).map((e) => e.project))]);
  const now = useNow(entries.some((e) => isActive(e.run)));
  const stages = Object.fromEntries(
    running.flatMap((e) => {
      const stage = (e.pending ? (operations ?? []).find((o) => o.id === e.run.id)?.progress : progress[e.project]?.[e.run.id]) ?? undefined;
      return stage === undefined ? [] : [[jobKey(e), stage] as const];
    }),
  );
  const format = useMemo(() => {
    const formatter = new Intl.DateTimeFormat(lang, { dateStyle: "short", timeStyle: "medium" });
    return (s: number) => formatter.format(new Date(s * 1000));
  }, [lang]);
  const rows = jobRows(entries, t, stages, now, format, places);
  const counts = summary(entries);
  const selected = entries.find((e) => jobKey(e) === route.query.job);
  const columns: Column[] = (["project", "analysis", "tags", "status", "started", "duration", "outcome"] as const).map((field) => ({
    field,
    title: t(`jobs.${field}`),
  }));
  const select = (key: string | undefined) => navigate({ ...route, query: { ...route.query, job: key ?? "" } });

  return (
    <div className="flex flex-col gap-5">
      <CenterTitle title="jobs.title" sub="jobs.sub" />
      <div className="flex flex-wrap items-end justify-between gap-4">
        <RadioField
          inline
          label="jobs.scope"
          value={scope}
          onChange={(v) => navigate({ ...route, query: { ...route.query, scope: v === "project" ? "project" : "", job: "" } })}
          options={[{ value: "all", label: "jobs.scope_all" }, ...(route.project === null ? [] : [{ value: "project", label: "jobs.scope_project" as const }])]}
        />
        <RadioField
          inline
          label="jobs.kind"
          value={kind}
          onChange={(v) => navigate({ ...route, query: { ...route.query, kind: v === "all" ? "" : v, job: "" } })}
          options={[
            { value: "all", label: "jobs.kind_all" },
            { value: "analysis", label: "jobs.kind_analysis" },
            { value: "manage", label: "jobs.kind_manage" },
          ]}
        />
        <p className="text-sm">{f("jobs.summary", counts.total, counts.running, counts.queued, counts.failed, duration(counts.computing))}</p>
      </div>
      {all.isSuccess && entries.length === 0 ? (
        <Callout k="jobs.empty" />
      ) : (
        <DataTable
          columns={columns}
          data={rows as Row[]}
          rowId="key"
          height="420px"
          selectable
          toolbar={false}
          selected={selected ? [jobKey(selected)] : []}
          onSelectionChange={(chosen) => {
            // A single selection: the row just checked.
            const keys = chosen.map((r) => String(r.key)).filter((k) => k !== route.query.job);
            select(keys[0] ?? (chosen.length === 0 ? undefined : route.query.job));
          }}
        />
      )}
      <SectionCard title="jobs.details" icon="logs">
        {selected === undefined ? (
          <p className="text-sm">
            <T k="jobs.select" />
          </p>
        ) : (
          <JobDetails
            key={jobKey(selected)}
            entry={selected}
            stage={stages[jobKey(selected)]}
            place={places[jobKey(selected)]}
            now={now}
            onSelect={(key) => select(key)}
          />
        )}
      </SectionCard>
    </div>
  );
}
