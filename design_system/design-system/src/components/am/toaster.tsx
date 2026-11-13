// Notifications ("toasts"): what ended while the user did something else —
// a computation, an import, an archive, and a job just queued. Top right,
// under the header, in the order given (newest first), each with an action
// (open the job, the data) and a close button. The list and its lifetime
// belong to the caller: this only draws it.
import { useI18n } from "#ui/i18n";
import { cn } from "#ui/lib/utils";
import { icons } from "./icons";

export interface Toast {
  id: string;
  /** `failure` stays until closed and is announced at once (alert). */
  kind: "success" | "failure" | "info";
  title: string;
  text?: string;
  action?: { label: string; onClick: () => void };
}

const accent: Record<Toast["kind"], string> = {
  success: "border-l-callout",
  failure: "border-l-danger",
  info: "border-l-panel-border",
};

export function Toaster({ toasts, onDismiss }: { toasts: readonly Toast[]; onDismiss: (id: string) => void }) {
  const { t } = useI18n();
  if (toasts.length === 0) return null;
  return (
    <div className="fixed top-[calc(var(--header-height,50px)+0.5rem)] right-4 z-50 flex max-h-[calc(100vh-var(--header-height,50px)-1rem)] w-[420px] max-w-[calc(100vw-2rem)] flex-col gap-2 overflow-y-auto">
      {toasts.map((toast) => {
        const Icon = toast.kind === "failure" ? icons.warning : toast.kind === "success" ? icons.validation : icons.info;
        return (
          <div
            key={toast.id}
            role={toast.kind === "failure" ? "alert" : "status"}
            className={cn(
              "flex shrink-0 gap-3 border border-l-4 border-panel-border bg-background px-4 py-3 text-sm text-foreground shadow-lg",
              accent[toast.kind],
            )}
          >
            <Icon className={cn("mt-0.5 size-5 shrink-0", toast.kind === "failure" ? "text-danger" : toast.kind === "success" ? "text-link" : "text-foreground")} />
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <b className="text-base break-words">{toast.title}</b>
              {toast.text && <span className="line-clamp-4 break-words text-muted-foreground" title={toast.text}>{toast.text}</span>}
              {toast.action && (
                <button type="button" className="self-start text-link underline hover:no-underline" onClick={toast.action.onClick}>
                  {toast.action.label}
                </button>
              )}
            </div>
            <button type="button" aria-label={t("toast.close")} className="self-start p-0.5 text-muted-foreground hover:text-foreground" onClick={() => onDismiss(toast.id)}>
              <icons.close className="size-4" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
