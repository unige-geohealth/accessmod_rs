// The details of a job, in Jobs and in the job panel: summary, progress,
// issues, inputs and outputs, and what can be done with it. `brief` (the
// panel) keeps what follows the job: progress, outcome, issues, pause and
// cancel.
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { CheckboxField } from "@accessmod/ui/am/form";
import { AmButton, LinkActions, MessageList } from "@accessmod/ui/am/messages";
import { T, useI18n } from "@accessmod/ui/i18n";
import { Progress } from "@accessmod/ui/ui/progress";
import { useState } from "react";
import { api, issuesOf } from "../api/client";
import { keys, queryClient, useDatasets } from "../api/queries";
import { navigate } from "../app/route";
import type { Api } from "../generated/types";
import { datasetLabel } from "../state/catalogue";
import { analysisLabel, duration, isActive, type JobItem, jobKey, seconds } from "../state/jobs";
import { percent, stageLabel } from "../state/run";
import { downloadJson, useFmt } from "./common";


export function JobDetails({
  entry,
  stage,
  place,
  now,
  onSelect,
  brief = false,
}: {
  entry: JobItem;
  stage: Api.Progress | undefined;
  place: Api.QueueEntry | undefined;
  now: number;
  onSelect: (key: string | undefined) => void;
  brief?: boolean;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const { project, run } = entry;
  const datasets = useDatasets(entry.pending || brief ? null : project).data;
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
        <b>{analysisLabel(t, run.analysis)}</b> · {entry.name}
        {run.tags.length > 0 && ` · [${run.tags.join(" ")}]`} · {t(`jobs.status_${run.status}`)} · {duration(seconds(run, now))}
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
      {!brief &&
        (
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
      {!brief && (
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
      )}
      {isActive(run) && (
        <div className="flex gap-2">
          {!entry.pending && (
            <AmButton block k={place?.paused ? "jobs.resume" : "jobs.pause"} onClick={() => void hold(!place?.paused)} />
          )}
          <AmButton block k="analysis.cancel" onClick={() => void cancel()} />
        </div>
      )}
      {!brief && !isActive(run) && !entry.pending && (
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
