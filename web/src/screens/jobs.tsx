// Jobs (in place of AM5 Logs): the task manager. The analyses and the
// operations (imports, exports, archives, project creation) of every
// project, newest first, with their progress, place in the queue and
// pause; a summary line; the details of the selected job: issues, inputs,
// outputs, Pause / Resume, Cancel, Retry, Export job, Remove. The
// selection, the scope and the kind are in the URL.
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { RadioField } from "@accessmod/ui/am/form";
import { AmButton, Callout, LinkActions, MessageList } from "@accessmod/ui/am/messages";
import { CenterTitle, SectionCard } from "@accessmod/ui/am/section";
import { T, useI18n } from "@accessmod/ui/i18n";
import { Progress } from "@accessmod/ui/ui/progress";
import { useEffect, useMemo, useState } from "react";
import { api, issuesOf } from "../api/client";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { CheckboxField } from "@accessmod/ui/am/form";
import { keys, queryClient, useAllRuns, useDatasets, useOperations, useProgressOf, useQueue } from "../api/queries";
import { navigate, type Route } from "../app/route";
import type { Api } from "../generated/types";
import { datasetLabel } from "../state/catalogue";
import {
  analysisLabel,
  duration,
  isActive,
  type JobItem,
  jobKey,
  jobKind,
  jobRows,
  newestFirst,
  ofKind,
  pendingJobs,
  queuePlaces,
  seconds,
  summary,
} from "../state/jobs";
import { percent, stageLabel } from "../state/run";
import { downloadJson, useFmt } from "./common";

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

/** Seconds since the epoch, ticking while a run is active (durations of running jobs). */
function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    if (!ticking) return;
    const timer = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(timer);
  }, [ticking]);
  return now;
}

function JobDetails({
  entry,
  stage,
  place,
  now,
  onSelect,
}: {
  entry: JobItem;
  stage: Api.Progress | undefined;
  place: Api.QueueEntry | undefined;
  now: number;
  onSelect: (key: string | undefined) => void;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const { project, run } = entry;
  const datasets = useDatasets(entry.pending ? null : project).data;
  const [failure, setFailure] = useState<string[]>([]);
  const [removing, setRemoving] = useState(false);
  const [withOutputs, setWithOutputs] = useState(false);
  const byId = new Map((datasets ?? []).map((d) => [d.id, d]));
  const analysis = (run.kind ?? "analysis") === "analysis";
  const act = async (action: () => Promise<unknown>) => {
    setFailure([]);
    try {
      await action();
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };
  const cancel = () =>
    act(() => (entry.pending ? api.command("operation.cancel", { id: run.id }) : api.command("run.cancel", { project, id: run.id })));
  const hold = (paused: boolean) => act(() => api.command(paused ? "run.pause" : "run.resume", { project, id: run.id }));
  const retry = () =>
    act(async () => {
      const again = await api.command("run.retry", { project, id: run.id });
      await queryClient.invalidateQueries({ queryKey: keys.allRuns });
      onSelect(jobKey({ project, run: again }));
    });
  const remove = () =>
    act(async () => {
      await api.command("run.remove", { project, id: run.id, outputs: withOutputs });
      await queryClient.invalidateQueries({ queryKey: keys.allRuns });
      await queryClient.invalidateQueries({ queryKey: keys.datasets(project) });
      onSelect(undefined);
    });
  const value = percent(stage);
  return (
    <div className="flex flex-col gap-2.5 text-sm">
      <p>
        <b>{analysisLabel(t, run.analysis)}</b> · {entry.name} · [{run.tags.join(" ")}] · {t(`jobs.status_${run.status}`)} · {duration(seconds(run, now))}
      </p>
      {run.status === "running" && (
        <div className="flex flex-col gap-1.5" role="status">
          <Progress value={value ?? 100} className={value === undefined ? "animate-pulse" : undefined} />
          {stage && <span className="text-xs text-muted-foreground">{f("analysis.running", stageLabel(t, stage.stage))}</span>}
        </div>
      )}
      {place?.paused && <p className="text-xs text-muted-foreground">{t("jobs.paused_note")}</p>}
      {run.outcome && <p>{run.outcome}</p>}
      {(run.error?.issues.length ?? 0) > 0 && <MessageList messages={(run.error?.issues ?? []).map((text) => ({ kind: "issue" as const, text }))} />}
      {failure.length > 0 && <MessageList messages={failure.map((text) => ({ kind: "issue" as const, text }))} />}
      {(
        [
          ["jobs.inputs", run.inputs],
          ["jobs.outputs", run.datasets],
        ] as const
      ).map(
        ([k, ids]) =>
          ids.length > 0 && (
            <div key={k}>
              <b>
                <T k={k} />
              </b>
              <ul className="list-disc pl-10">
                {ids.map((id) => {
                  const d = byId.get(id);
                  return <li key={id}>{d ? datasetLabel(t, d) : `#${id}`}</li>;
                })}
              </ul>
            </div>
          ),
      )}
      <LinkActions
        actions={[
          ...(analysis
            ? [
                {
                  icon: "download" as const,
                  k: "analysis.download_job" as const,
                  onClick: () => downloadJson(`${run.analysis}-${project}-${run.id}`, run.job),
                },
              ]
            : []),
          // Data lists the job, inputs and outputs of finished runs.
          ...(run.status === "done" && analysis
            ? [
                {
                  icon: "data" as const,
                  k: "jobs.open_data" as const,
                  onClick: () => navigate({ project, section: "data", panel: "filter", query: { run: String(run.id) } }),
                },
              ]
            : []),
        ]}
      />
      {isActive(run) && (
        <div className="flex gap-2">
          {!entry.pending && (
            <AmButton block k={place?.paused ? "jobs.resume" : "jobs.pause"} onClick={() => void hold(!place?.paused)} />
          )}
          <AmButton block k="analysis.cancel" onClick={() => void cancel()} />
        </div>
      )}
      {!isActive(run) && !entry.pending && (
        <div className="flex gap-2">
          {analysis && run.status !== "done" && <AmButton block icon="reset" k="jobs.retry" onClick={() => void retry()} />}
          <AmButton block variant="danger" k="jobs.remove" onClick={() => setRemoving(true)} />
        </div>
      )}
      <ConfirmDialog open={removing} onOpenChange={setRemoving} confirm="jobs.remove" onConfirm={() => void remove()}>
        <p className="mb-2">{f("jobs.remove_confirm", run.id)}</p>
        {run.datasets.length > 0 && (
          <CheckboxField label="jobs.remove_outputs" checked={withOutputs} onChange={setWithOutputs} />
        )}
      </ConfirmDialog>
    </div>
  );
}
