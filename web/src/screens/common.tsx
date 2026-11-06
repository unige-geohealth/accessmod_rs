// Pieces shared by the screens: AM5 texts with placeholders, process
// feedback (transfers, batches), summary lists.
import { type Message, MessageList } from "@accessmod/ui/am/messages";
import { type Key, T, useI18n } from "@accessmod/ui/i18n";
import { AmButton } from "@accessmod/ui/am/messages";
import { Progress } from "@accessmod/ui/ui/progress";
import { api } from "../api/client";
import { useOperations, useProgress, useQueue } from "../api/queries";
import type { TaskRef } from "../api/tasks";
import type { Api } from "../generated/types";
import { percent, stageLabel } from "../state/run";
import type { Batch } from "../state/batch";
import type { Entry } from "../state/project-summary";
import type { Transfer } from "../state/transfer";

/** An AM5 text with `%s` / `%1$s` placeholders filled in order. */
export function fmt(text: string, ...args: (string | number)[]): string {
  let next = 0;
  return text
    .replace(/%(\d+)\$s|%s/g, (_, n: string | undefined) => String(args[n === undefined ? next++ : Number(n) - 1] ?? ""))
    .replace(/%%/g, "%");
}

/** Save `value` as a JSON file (`name`.json) through the browser. */
export function downloadJson(name: string, value: unknown) {
  const blob = new Blob([`${JSON.stringify(value, null, 2)}\n`], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = window.document.createElement("a");
  a.href = url;
  a.download = `${name}.json`;
  a.click();
  URL.revokeObjectURL(url);
}

export function useFmt() {
  const { t } = useI18n();
  return (key: Key, ...args: (string | number)[]) => fmt(t(key), ...args);
}

/** Progress, then the outcome, of an upload followed by a command. */
export function TransferStatus({ transfer, doneKey }: { transfer: Transfer; doneKey: Key }) {
  const f = useFmt();
  switch (transfer.state) {
    case "idle":
      return null;
    case "uploading": {
      const percent = Math.round(transfer.fraction * 100);
      return (
        <div className="flex flex-col gap-1" role="status">
          <Progress value={percent} />
          <span className="text-xs text-muted-foreground">{f("upload.sending", percent)}</span>
        </div>
      );
    }
    case "processing":
      return transfer.task === null ? (
        <div className="flex flex-col gap-1" role="status">
          <Progress value={100} className="animate-pulse" />
          <span className="text-xs text-muted-foreground">
            <T k="project_import_progress_text" />
          </span>
        </div>
      ) : (
        <TaskProgress task={transfer.task} />
      );
    case "done":
      return <MessageList messages={[{ kind: "info", text: f(doneKey, transfer.label) }]} />;
    case "failed":
      return <MessageList messages={transfer.issues.map((text) => ({ kind: "issue", text }))} />;
  }
}

/** What a queued or running task is doing, from events. */
export interface TaskView {
  status: Api.RunStatus;
  /** Tasks before it in its lane (0: running). */
  position: number;
  paused: boolean;
  progress: Api.Progress | undefined;
}

/** The view of `task` from the queue, progress and operations (selector). */
export function taskView(
  task: TaskRef,
  queue: readonly Api.QueueEntry[],
  progress: Readonly<Record<number, Api.Progress>> | undefined,
  operations: readonly Api.PendingOperation[],
): TaskView {
  const entry =
    task.kind === "run"
      ? queue.find((e) => e.project === task.project && e.run === task.id)
      : queue.find((e) => e.project === ":pending" && e.run === task.id);
  const operation = task.kind === "operation" ? operations.find((o) => o.id === task.id) : undefined;
  const position = entry?.position ?? 0;
  return {
    status: operation?.status ?? (entry === undefined ? "running" : position === 0 ? "running" : "queued"),
    position,
    paused: entry?.paused ?? false,
    progress: task.kind === "run" ? progress?.[task.id] : (operation?.progress ?? undefined),
  };
}

/** Progress of a queued task: its stage and percent, or its place in the queue; Cancel. */
export function TaskProgress({ task }: { task: TaskRef }) {
  const { t } = useI18n();
  const f = useFmt();
  const queue = useQueue().data ?? [];
  const progress = useProgress(task.kind === "run" ? task.project : null).data;
  const operations = useOperations().data ?? [];
  const view = taskView(task, queue, progress, operations);
  const value = percent(view.progress);
  const stageText = view.progress === undefined ? t("task.starting") : stageLabel(t, view.progress.stage);
  const text = view.paused
    ? t("task.paused")
    : view.position > 0
      ? f("task.waiting", view.position - 1)
      : value === undefined
        ? stageText
        : f("task.stage_percent", stageText, value);
  const cancel = () =>
    void (task.kind === "run" ? api.command("run.cancel", { project: task.project, id: task.id }) : api.command("operation.cancel", { id: task.id })).catch(() => {
      // Already over: its end arrives as an event.
    });
  return (
    <div className="flex flex-col gap-1" role="status">
      <Progress value={value ?? 100} className={value === undefined ? "animate-pulse" : undefined} />
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs text-muted-foreground">{text}</span>
        <AmButton className="px-2 py-0.5 text-xs" k="task.cancel" onClick={cancel} />
      </div>
    </div>
  );
}

/** Progress, then the outcome of each item, of a batch of commands. */
export function BatchStatus({ batch, doneKey }: { batch: Batch; doneKey: Key }) {
  const f = useFmt();
  switch (batch.state) {
    case "idle":
      return null;
    case "running":
      return <Progress value={(batch.done / batch.total) * 100} role="status" />;
    case "done": {
      const messages: Message[] = batch.outcomes.flatMap((o): Message[] =>
        o.issues === null ? [{ kind: "info", text: f(doneKey, o.label) }] : o.issues.map((i) => ({ kind: "issue", text: `${o.label}: ${i}` })),
      );
      return <MessageList messages={messages} />;
    }
  }
}

/** AM5 summary list in a well: GRASS names underlined, values in monospace. */
export function EntryList({ entries }: { entries: Entry[] }) {
  return (
    <div className="border border-well-border bg-well p-[15px]">
      <dl className="ml-[35px]">
        {entries.map(([name, value]) => (
          <div key={name} className="mb-1.5">
            <dt className="underline">{name}</dt>
            <dd className="ml-0 list-item list-inside list-disc font-mono">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
