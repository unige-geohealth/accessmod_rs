// Pieces shared by the screens: AM5 texts with placeholders, process
// feedback (transfers, batches), summary lists.
import { type Message, MessageList } from "@accessmod/ui/am/messages";
import { type Key, T, useI18n } from "@accessmod/ui/i18n";
import { Progress } from "@accessmod/ui/ui/progress";
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

/**
 * What precedes the job of an upload followed by a command: the upload,
 * the request, or its refusal. The job itself is notified, queued and
 * ended (docs/ui.md, "Notifications").
 */
export function TransferStatus({ transfer }: { transfer: Transfer }) {
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
      ) : null;
    case "done":
      return null;
    case "failed":
      return <MessageList messages={transfer.issues.map((text) => ({ kind: "issue", text }))} />;
  }
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
