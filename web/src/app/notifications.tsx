// The toaster of the app: the notices of jobs that ended, each opening its
// job (or, for a done analysis, its data; for a made project, the project).
import { type Toast, Toaster } from "@accessmod/ui/am/toaster";
import { type Key, useI18n } from "@accessmod/ui/i18n";
import { followed, isNoticeOf, jobPanel } from "../state/job-panel";
import { analysisLabel } from "../state/jobs";
import { type Notice, notifications, visibleNotices } from "../state/notifications";
import { useStore } from "../state/store";
import { useFmt, useNow } from "../screens/common";
import { HOME, navigate } from "./route";

/** Where a job's notice (or the job panel, at its end) leads: the project it made, its data, or the job. */
export function jobAction(
  t: (key: Key) => string,
  n: Pick<Notice, "pending" | "made" | "project" | "run" | "kind" | "status">,
): { label: string; onClick: () => void } {
  if (n.pending) {
    return n.made
      ? { label: t("toast.open_project"), onClick: () => navigate({ ...HOME, project: n.project }) }
      : { label: t("toast.open_job"), onClick: () => navigate({ ...HOME, project: null, section: "jobs", query: { job: `:pending/${n.run}` } }) };
  }
  if (n.kind === "analysis" && n.status === "done") {
    return { label: t("toast.open_data"), onClick: () => navigate({ project: n.project, section: "data", panel: "filter", query: { run: String(n.run) } }) };
  }
  return { label: t("toast.open_job"), onClick: () => navigate({ ...HOME, project: n.project, section: "jobs", query: { job: `${n.project}/${n.run}` } }) };
}

const KIND: Record<string, Toast["kind"]> = { done: "success", failed: "failure", interrupted: "failure" };

export function Notifications() {
  const { t } = useI18n();
  const f = useFmt();
  const state = useStore(notifications, (s) => s);
  // A clock behind a notice that just arrived only shows it longer.
  const now = useNow(state.notices.length > state.closed.length);
  // The job the panel shows is told there, not twice.
  const shown = useStore(jobPanel, followed);
  const toasts: Toast[] = visibleNotices(state, now)
    .filter((n) => shown === null || !isNoticeOf(n, shown))
    .map((n) => ({
    id: n.id,
    kind: KIND[n.status] ?? "info",
    title: f("toast.title", analysisLabel(t, n.analysis), t(`jobs.status_${n.status}` as Key)),
    ...(n.text === "" ? {} : { text: n.text }),
    action: jobAction(t, n),
  }));
  return <Toaster toasts={toasts} onDismiss={(id) => notifications.dispatch({ type: "dismiss", id })} />;
}
