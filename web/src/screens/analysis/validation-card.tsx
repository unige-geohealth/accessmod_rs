// AM5 "Validation" card of an analysis tab: what is missing, the server's
// quick check (it alone gates Compute, with its warnings), the outputs and
// Compute. Compute queues a job and says so; the form stays editable, so
// another scenario is one change and one click away. The job's progress,
// pause and cancel live in Jobs; its end is a notification.
import { AmButton, LinkActions, type Message, MessageList } from "@accessmod/ui/am/messages";
import { SectionCard } from "@accessmod/ui/am/section";
import { type Key, T, useI18n } from "@accessmod/ui/i18n";
import { useState } from "react";
import { api, issuesOf } from "../../api/client";
import { keys, queryClient, useQuickCheck, useRuns } from "../../api/queries";
import type { Api } from "../../generated/types";
import { classLabel, plannedOutputs } from "../../state/catalogue";
import { type Issue, sessionStore } from "../../state/drafts";
import { usePreference } from "../../state/preferences";
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
  /** AM5 Zonal statistics: "Update". */
  button?: Key;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const message = (kind: Message["kind"], i: Issue): Message => (typeof i === "string" ? { kind, k: i } : { kind, text: f(i.key, ...i.args) });
  const session = sessionStore(project, analysis);
  const { run: started, starting } = useStore(session, (s) => s);
  const runs = useRuns(project).data;
  const quick = useQuickCheck(project, analysis, document);
  const [failure, setFailure] = useState<string[]>([]);
  const overwrite = usePreference("overwrite");
  const outputs = plannedOutputs(analysis, tags, datasets.values(), runs);
  const taken = outputs.some((o) => o.existing !== undefined);
  const reservers = [...new Set(outputs.flatMap((o) => (o.reservedBy === undefined ? [] : [o.reservedBy])))];
  const reserved = reservers.length > 0;
  const ownJob = reservers.length === 1 && reservers[0] === started;

  const messages: Message[] = [
    ...issues.map((i) => message("issue", i)),
    ...notices.map((i) => message("info", i)),
    ...(quick.isError ? issuesOf(quick.error, t).map((text) => ({ kind: "issue" as const, text })) : []),
    ...(quick.data?.warnings ?? []).map((w) => ({ kind: "info" as const, text: w.message })),
    ...failure.map((text) => ({ kind: "issue" as const, text })),
    ...(taken && !overwrite ? [{ kind: "issue" as const, k: "analysis.outputs_taken" as const }] : []),
    // The job this tab just queued: its notices say it, not the form.
    ...(reserved && !ownJob ? [{ kind: "issue" as const, k: "analysis.outputs_reserved" as const }] : []),
  ];
  const ready = document !== null && quick.isSuccess && !starting && (overwrite || !taken) && !reserved;

  const compute = async () => {
    setFailure([]);
    if (document === null) return;
    session.dispatch({ type: "starting" });
    try {
      const record = await api.command("run.start", { project, analysis, document, tags, overwrite });
      session.dispatch({ type: "started", run: record.id });
      await queryClient.invalidateQueries({ queryKey: keys.runs(project) });
    } catch (error) {
      session.dispatch({ type: "start_failed" });
      setFailure(issuesOf(error, t));
    }
  };

  return (
    <SectionCard title="analysis_validation_title" sub="analysis_validation_sub" icon="validation">
      {messages.length > 0 ? (
        <MessageList messages={messages} />
      ) : (
        <p className="text-sm">
          <T k={quick.isFetching ? "analysis.validating" : "analysis.valid"} />
        </p>
      )}
      {outputs.length > 0 && (
        <div className="text-sm">
          <b>
            <T k="srv_analysis_accessibility_validation_output_dataset" />
          </b>
          <ul>
            {outputs.map((o) => (
              <li key={o.class}>
                {classLabel(t, o.class)} [{tags.join(" ")}]{" "}
                {o.reservedBy !== undefined ? (
                  <b className="text-warning">({f("analysis.output_reserved", o.reservedBy)})</b>
                ) : o.existing === undefined ? (
                  <span className="text-muted-foreground">({t("analysis.output_ok")})</span>
                ) : (
                  <b className="text-warning">({t("analysis.output_overwrite")})</b>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      {/* Red while something blocks it, primary once it can run. */}
      <AmButton variant={ready ? "primary" : "danger"} block disabled={!ready} k={button} onClick={() => void compute()} />
      {document !== null && <LinkActions actions={[{ icon: "download", k: "analysis.download_job", onClick: () => downloadJson(analysis, document) }]} />}
    </SectionCard>
  );
}
