// The job panel (AM5 progress modal): the upload, then the job the user
// just started in this window, until its end. It can be closed at any
// time: the job goes on, in Jobs (docs/ui.md, "Notifications"). The job is
// read from the caches the events keep, never copied.
import { AmButton } from "@accessmod/ui/am/messages";
import { T, useI18n } from "@accessmod/ui/i18n";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@accessmod/ui/ui/dialog";
import { Progress } from "@accessmod/ui/ui/progress";
import { useOperations, useProgress, useProject, useQueue, useRuns } from "../api/queries";
import type { TaskRef } from "../api/tasks";
import type { Api } from "../generated/types";
import { useFmt, useNow } from "../screens/common";
import { JobDetails } from "../screens/job-details";
import { isNoticeOf, jobPanel } from "../state/job-panel";
import { analysisLabel, isActive, type JobItem, jobKey, operationJob, queuePlaces } from "../state/jobs";
import { notifications } from "../state/notifications";
import { useStore } from "../state/store";
import { jobAction } from "./notifications";
import { HOME, navigate } from "./route";

/** Close the panel; the notices of the job it showed are seen. */
function close(task: TaskRef | null) {
  if (task !== null) {
    for (const n of notifications.getState().notices) {
      if (isNoticeOf(n, task)) notifications.dispatch({ type: "dismiss", id: n.id });
    }
  }
  jobPanel.dispatch({ type: "close" });
}

export function JobPanel() {
  const state = useStore(jobPanel, (s) => s);
  const task = state.kind === "job" ? state.task : null;
  return (
    <Dialog open={state.kind !== "closed"} onOpenChange={(open) => !open && close(task)}>
      <DialogContent showCloseButton={false} className="rounded-none sm:max-w-xl">
        {state.kind === "upload" && <UploadView fraction={state.fraction} />}
        {task !== null && <JobView key={task.kind === "run" ? `${task.project}/${task.id}` : `:pending/${task.id}`} task={task} />}
      </DialogContent>
    </Dialog>
  );
}

function UploadView({ fraction }: { fraction: number }) {
  const f = useFmt();
  const percent = Math.round(fraction * 100);
  return (
    <>
      <DialogTitle className="text-lg font-medium">{f("upload.sending", percent)}</DialogTitle>
      <DialogDescription asChild>
        <div className="flex flex-col gap-2.5 text-sm text-foreground" role="status">
          <Progress value={percent} />
          <p className="text-xs text-muted-foreground">
            <T k="job_panel.upload_note" />
          </p>
        </div>
      </DialogDescription>
      <AmButton block k="job_panel.background" onClick={() => close(null)} />
    </>
  );
}

/** The job of `task`, as the events keep it: a run of a project, or an operation. */
function useJob(task: TaskRef): { entry: JobItem | undefined; stage: Api.Progress | undefined; made: boolean } {
  const project = task.kind === "run" ? task.project : null;
  const runs = useRuns(project).data;
  const progress = useProgress(project).data;
  const name = useProject(project).data?.name;
  const operations = useOperations().data;
  if (task.kind === "run") {
    const run = runs?.find((r) => r.id === task.id);
    return { entry: run && { project: task.project, name: name ?? task.project, run }, stage: progress?.[task.id], made: false };
  }
  const operation = operations?.find((o) => o.id === task.id);
  return { entry: operation && operationJob(operation), stage: operation?.progress ?? undefined, made: operation?.recorded === true };
}

function JobView({ task }: { task: TaskRef }) {
  const { t } = useI18n();
  const f = useFmt();
  const { entry, stage, made } = useJob(task);
  const queue = useQueue().data;
  const now = useNow(entry !== undefined && isActive(entry.run));
  if (entry === undefined) {
    // Not listed (yet: just queued; or no longer: an operation recorded
    // in its project leaves the list on a server restart). Jobs has it.
    const jobs = { ...HOME, project: task.kind === "run" ? task.project : null, section: "jobs" as const, query: { job: task.kind === "run" ? `${task.project}/${task.id}` : `:pending/${task.id}` } };
    return (
      <>
        <DialogTitle className="text-lg font-medium">
          <T k="job_panel.waiting" />
        </DialogTitle>
        <DialogDescription asChild>
          <div role="status">
            <Progress value={100} className="animate-pulse" />
          </div>
        </DialogDescription>
        <div className="flex gap-2">
          <AmButton
            block
            k="toast.open_job"
            onClick={() => {
              close(task);
              navigate(jobs);
            }}
          />
          <AmButton block k="job_panel.background" onClick={() => close(task)} />
        </div>
      </>
    );
  }
  const { run } = entry;
  const active = isActive(run);
  const action = jobAction(t, { pending: entry.pending === true, made, project: entry.project, run: run.id, kind: run.kind ?? "analysis", status: run.status });
  return (
    <>
      <DialogTitle className="text-lg font-medium">{f("toast.title", analysisLabel(t, run.analysis), t(`jobs.status_${run.status}`))}</DialogTitle>
      <DialogDescription asChild>
        <div className="text-foreground">
          <JobDetails brief entry={entry} stage={stage} place={queuePlaces(queue ?? [])[jobKey(entry)]} now={now} onSelect={() => undefined} />
        </div>
      </DialogDescription>
      {active ? (
        <AmButton block k="job_panel.background" onClick={() => close(task)} />
      ) : (
        <div className="flex gap-2">
          <AmButton
            block
            onClick={() => {
              close(task);
              action.onClick();
            }}
          >
            {action.label}
          </AmButton>
          <AmButton block k="job_panel.close" onClick={() => close(task)} />
        </div>
      )}
    </>
  );
}
