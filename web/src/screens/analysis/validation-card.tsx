// AM5 "Validation" card of an analysis tab: what is missing, the server's
// quick check (it alone gates Compute), the pre-analysis of the inputs
// (`prepare`, a task with its progress: Compute does not wait for it),
// Compute, progress, Cancel, the "Process finished" dialog and Export job.
import { MessageDialog } from "@accessmod/ui/am/message-dialog";
import { AmButton, LinkActions, type Message, MessageList } from "@accessmod/ui/am/messages";
import { SectionCard } from "@accessmod/ui/am/section";
import { type Key, T, useI18n } from "@accessmod/ui/i18n";
import { Progress } from "@accessmod/ui/ui/progress";
import { useState } from "react";
import { api, issuesOf } from "../../api/client";
import { keys, queryClient, useCheckState, usePreAnalysis, useProgress, useQuickCheck, useRuns } from "../../api/queries";
import type { Api } from "../../generated/types";
import { classLabel, plannedOutputs } from "../../state/catalogue";
import { type Issue, sessionStore } from "../../state/drafts";
import { usePreference } from "../../state/preferences";
import { busy, minutes, percent, runView, type RunView } from "../../state/run";
import { useStore } from "../../state/store";
import { downloadJson, useFmt } from "../common";

export function ValidationCard({
  project,
  analysis,
  issues,
  notices,
  document,
  tags,
  datasets,
  button = "analysis_validation_compute_btn",
  dialog = true,
}: {
  project: string;
  analysis: string;
  issues: Issue[];
  /** AM5 information messages (not blocking). */
  notices: Issue[];
  /** Complete job document; `null` while incomplete. */
  document: Record<string, unknown> | null;
  tags: string[];
  datasets: ReadonlyMap<number, Api.Dataset>;
  /** AM5 Zonal statistics: "Update", and the result shown in place of the dialog. */
  button?: Key;
  dialog?: boolean;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const message = (kind: Message["kind"], i: Issue): Message => (typeof i === "string" ? { kind, k: i } : { kind, text: f(i.key, ...i.args) });
  const quick = useQuickCheck(project, analysis, document);
  const pre = usePreAnalysis(project, analysis, document, quick.isSuccess);
  const checkState = useCheckState(project, analysis).data ?? null;
  const session = sessionStore(project, analysis);
  const { run, acknowledged } = useStore(session, (s) => s);
  const runs = useRuns(project).data;
  const progress = useProgress(project).data;
  const view: RunView = runView(runs, progress, run);
  const [failure, setFailure] = useState<string[]>([]);
  const overwrite = usePreference("overwrite");
  const outputs = plannedOutputs(analysis, tags, datasets.values());
  const taken = outputs.filter((o) => o.existing !== undefined);

  const messages: Message[] = [
    ...issues.map((i) => message("issue", i)),
    ...notices.map((i) => message("info", i)),
    ...(quick.isError ? issuesOf(quick.error, t).map((text) => ({ kind: "issue" as const, text })) : []),
    ...(pre.isError ? issuesOf(pre.error, t).map((text) => ({ kind: "issue" as const, text })) : []),
    ...((pre.data?.warnings as { message: string }[] | undefined) ?? []).map((w) => ({ kind: "info" as const, text: w.message })),
    ...failure.map((text) => ({ kind: "issue" as const, text })),
    ...(taken.length > 0 && !overwrite ? [{ kind: "issue" as const, k: "analysis.outputs_taken" as const }] : []),
  ];
  if (view.state === "failed") {
    const key = view.record.status === "cancelled" ? "analysis.run_cancelled" : "analysis.run_failed";
    messages.push({ kind: "issue", k: key }, ...(view.record.error?.issues ?? []).map((text) => ({ kind: "issue" as const, text })));
  }
  // The quick check gates Compute; a pre-analysis that found issues too (the run would fail on them).
  const ready = document !== null && quick.isSuccess && !pre.isError && !busy(view) && (overwrite || taken.length === 0);
  const prechecking = quick.isSuccess && pre.isFetching;

  const compute = async () => {
    setFailure([]);
    try {
      if (document === null) return;
      const record = await api.command("run.start", { project, analysis, document, tags, overwrite });
      await queryClient.invalidateQueries({ queryKey: keys.runs(project) });
      session.dispatch({ type: "started", run: record.id });
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };
  const cancel = async () => {
    if (run === null) return;
    try {
      await api.command("run.cancel", { project, id: run });
    } catch (error) {
      setFailure(issuesOf(error, t));
    }
  };

  return (
    <SectionCard title="analysis_validation_title" sub="analysis_validation_sub" icon="validation">
      {messages.length > 0 ? (
        <MessageList messages={messages} />
      ) : (
        <p className="text-sm">
          <T k={quick.isFetching || prechecking ? "analysis.validating" : "analysis.valid"} />
        </p>
      )}
      {prechecking && <PrecheckProgress check={checkState} computable={ready} />}
      {outputs.length > 0 && (
        <div className="text-sm">
          <b>
            <T k="srv_analysis_accessibility_validation_output_dataset" />
          </b>
          <ul>
            {outputs.map((o) => (
              <li key={o.class}>
                {classLabel(t, o.class)} [{tags.join(" ")}]{" "}
                {o.existing === undefined ? (
                  <span className="text-muted-foreground">({t("analysis.output_ok")})</span>
                ) : (
                  <b className="text-warning">({t("analysis.output_overwrite")})</b>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      {busy(view) ? (
        <RunProgress view={view} onCancel={() => void cancel()} />
      ) : (
        <AmButton variant="danger" block disabled={!ready} k={button} onClick={() => void compute()} />
      )}
      {document !== null && <LinkActions actions={[{ icon: "download", k: "analysis.download_job", onClick: () => downloadJson(analysis, document) }]} />}
      <MessageDialog
        open={dialog && view.state === "done" && !acknowledged}
        onOpenChange={(open) => {
          if (!open) session.dispatch({ type: "acknowledged" });
        }}
        title="srv_analysis_accessibility_process_finished_message"
      >
        {view.state === "done" && (
          <>
            <p>{f("srv_analysis_accessibility_process_finished_timing", minutes(view.record).toFixed(3))}</p>
            <ul className="list-disc pl-10">
              {view.record.datasets.map((id) => {
                const d = datasets.get(id);
                return <li key={id}>{d ? `${classLabel(t, d.class)} [${d.tags.join(" ")}]` : `#${id}`}</li>;
              })}
            </ul>
          </>
        )}
      </MessageDialog>
    </SectionCard>
  );
}

/** Progress of the pre-analysis: its stage and percent, while Compute is already offered. */
function PrecheckProgress({ check, computable }: { check: Api.Check | null; computable: boolean }) {
  const { t } = useI18n();
  const f = useFmt();
  const stage = check?.status === "running" ? (check.progress ?? undefined) : undefined;
  const value = percent(stage);
  const text = stage === undefined ? t("analysis.validating") : value === undefined ? stage.stage : f("task.stage_percent", stage.stage, value);
  return (
    <div className="flex flex-col gap-1" role="status">
      <Progress value={value ?? 100} className={value === undefined ? "animate-pulse" : undefined} />
      <span className="text-xs text-muted-foreground">{f("analysis.precheck", text)}</span>
      {computable && <span className="text-xs text-muted-foreground">{t("analysis.precheck_note")}</span>}
    </div>
  );
}

function RunProgress({ view, onCancel }: { view: RunView; onCancel: () => void }) {
  const f = useFmt();
  const stage = view.state === "running" ? view.progress : undefined;
  const value = percent(stage);
  return (
    <div className="flex flex-col gap-1.5" role="status">
      <Progress value={value ?? 100} className={value === undefined ? "animate-pulse" : undefined} />
      <span className="text-xs text-muted-foreground">{stage ? f("analysis.running", stage.stage) : f("analysis.queued")}</span>
      <AmButton block k="analysis.cancel" onClick={onCancel} />
    </div>
  );
}
