// Notifications ("toasts"): what ended while the user did something else —
// a computation, an import, an archive. Bottom right, newest at the bottom,
// each with an action (open the job, the data) and a close button. The
// list and its lifetime belong to the caller: this only draws it.
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
    <div className="fixed right-4 bottom-4 z-50 flex max-h-[calc(100vh-2rem)] w-[360px] max-w-[calc(100vw-2rem)] flex-col gap-2 overflow-y-auto">
      {toasts.map((toast) => {
        const Icon = toast.kind === "failure" ? icons.warning : toast.kind === "success" ? icons.validation : icons.info;
        return (
          <div
            key={toast.id}
            role={toast.kind === "failure" ? "alert" : "status"}
            className={cn(
              "flex shrink-0 gap-2 border border-l-4 border-panel-border bg-background p-3 text-sm text-foreground shadow-md",
              accent[toast.kind],
            )}
          >
            <Icon className={cn("mt-0.5 size-4 shrink-0", toast.kind === "failure" ? "text-danger" : "text-foreground")} />
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <b className="break-words">{toast.title}</b>
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
