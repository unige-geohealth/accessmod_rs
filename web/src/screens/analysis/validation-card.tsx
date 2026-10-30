// AM5 "Validation" card of an analysis tab: what is missing, the server's
// checks (`prepare`), Compute, progress, Cancel, the "Process finished"
// dialog and Export job.
import { MessageDialog } from "@accessmod/ui/am/message-dialog";
import { AmButton, LinkActions, type Message, MessageList } from "@accessmod/ui/am/messages";
import { SectionCard } from "@accessmod/ui/am/section";
import { T, useI18n } from "@accessmod/ui/i18n";
import { Progress } from "@accessmod/ui/ui/progress";
import { useState } from "react";
import { api, issuesOf } from "../../api/client";
import { keys, queryClient, useProgress, useRuns, useValidation } from "../../api/queries";
import type { Api } from "../../generated/types";
import { classLabel } from "../../state/catalogue";
import { type Issue, sessionStore } from "../../state/drafts";
import { busy, minutes, percent, runView, type RunView } from "../../state/run";
import { useStore } from "../../state/store";
import { useFmt } from "../common";

export function ValidationCard({
  project,
  analysis,
  issues,
  notices,
  document,
  tags,
  datasets,
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
}) {
  const { t } = useI18n();
  const f = useFmt();
  const message = (kind: Message["kind"], i: Issue): Message => (typeof i === "string" ? { kind, k: i } : { kind, text: f(i.key, ...i.args) });
  const validation = useValidation(project, analysis, document);
  const session = sessionStore(project, analysis);
  const { run, acknowledged } = useStore(session, (s) => s);
  const runs = useRuns(project).data;
  const progress = useProgress(project).data;
  const view: RunView = runView(runs, progress, run);
  const [failure, setFailure] = useState<string[]>([]);

  const messages: Message[] = [
    ...issues.map((i) => message("issue", i)),
    ...notices.map((i) => message("info", i)),
    ...(validation.isError ? issuesOf(validation.error, t).map((text) => ({ kind: "issue" as const, text })) : []),
    ...((validation.data?.warnings as { message: string }[] | undefined) ?? []).map((w) => ({ kind: "info" as const, text: w.message })),
    ...failure.map((text) => ({ kind: "issue" as const, text })),
  ];
  if (view.state === "failed") {
    const key = view.record.status === "cancelled" ? "analysis.run_cancelled" : "analysis.run_failed";
    messages.push({ kind: "issue", k: key }, ...(view.record.error?.issues ?? []).map((text) => ({ kind: "issue" as const, text })));
  }
  const ready = document !== null && validation.isSuccess && !busy(view);

  const compute = async () => {
    setFailure([]);
    try {
      if (document === null) return;
      const record = await api.command("run.start", { project, analysis, document, tags });
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
  const download = () => {
    const blob = new Blob([`${JSON.stringify(document, null, 2)}\n`], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = window.document.createElement("a");
    a.href = url;
    a.download = `${analysis}.json`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <SectionCard title="analysis_validation_title" sub="analysis_validation_sub" icon="validation">
      {messages.length > 0 ? (
        <MessageList messages={messages} />
      ) : (
        <p className="text-sm">
          <T k={validation.isFetching ? "analysis.validating" : "analysis.valid"} />
        </p>
      )}
      {busy(view) ? (
        <RunProgress view={view} onCancel={() => void cancel()} />
      ) : (
        <AmButton variant="danger" block disabled={!ready} k="analysis_validation_compute_btn" onClick={() => void compute()} />
      )}
      {document !== null && <LinkActions actions={[{ icon: "download", k: "analysis.download_job", onClick: download }]} />}
      <MessageDialog
        open={view.state === "done" && !acknowledged}
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
