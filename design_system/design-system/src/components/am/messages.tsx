import { cva, type VariantProps } from "class-variance-authority";
import { type ComponentProps, type ReactNode, useRef } from "react";
import { type Key, T } from "#ui/i18n";
import { cn } from "#ui/lib/utils";
import { type IconName, icons } from "./icons";

/** AM5 `.callout.callout-info`: solid blue block, white text ("Please add …"). */
export function Callout({ k, children, className }: { k?: Key; children?: ReactNode; className?: string }) {
  return (
    <p className={cn("mb-5 border-l-[5px] border-callout-border bg-callout px-[15px] py-[15px] text-callout-foreground", className)}>
      {k ? <T k={k} /> : children}
    </p>
  );
}

/** `issue` blocks the action (AM5 "Issue(s):"), `info` does not ("Information:"). */
export type Message = { kind: "issue" | "info"; k?: Key; text?: string };

const messageIcon: Record<Message["kind"], IconName> = { issue: "warning", info: "info" };
const headings: Record<Message["kind"], Key> = {
  issue: "srv_pop_correction_issues_text",
  info: "srv_analysis_accessibility_validation_information",
};

/** AM5 validation block: "Issue(s):", "Information:" lists with an icon per line. */
export function MessageList({ messages }: { messages: Message[] }) {
  const groups = (["issue", "info"] as const)
    .map((kind) => [kind, messages.filter((m) => m.kind === kind)] as const)
    .filter(([, list]) => list.length);
  return (
    <div className="flex flex-col gap-1">
      {groups.map(([kind, list]) => (
        <div key={kind}>
          <div className="font-bold">
            <T k={headings[kind]} />
          </div>
          <ul>
            {list.map((m, i) => {
              const Icon = icons[messageIcon[kind]];
              return (
                // biome-ignore lint/suspicious/noArrayIndexKey: static list
                <li key={i} className="flex gap-1">
                  <Icon className="mt-[3px] size-3.5 shrink-0" />
                  <span>{m.k ? <T k={m.k} /> : m.text}</span>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </div>
  );
}

/** Help/info bullet list (Data › Import "Validation information"). */
export function InfoList({ title, items }: { title?: Key; items: Key[] }) {
  return (
    <div>
      {title && (
        <div className="font-bold">
          <T k={title} />
        </div>
      )}
      <ul className="list-disc pl-10">
        {items.map((k) => (
          <li key={k}>
            <icons.info className="mr-1 inline size-4 -translate-y-px fill-foreground text-background" />
            <T k={k} />
          </li>
        ))}
      </ul>
    </div>
  );
}

/** AM5 link actions above tables: "↺ Reset | ✎ Import … | ?" (`.link-button`). */
export function LinkActions({
  actions,
}: {
  actions: { k?: Key; icon: IconName; onClick?: () => void; label?: string }[];
}) {
  return (
    <div className="mb-2.5 flex flex-wrap items-center gap-1 text-link">
      {actions.map((a, i) => {
        const Icon = icons[a.icon];
        return (
          <span key={a.k ?? a.icon} className="inline-flex items-center gap-1">
            {i > 0 && <span className="text-foreground">|</span>}
            <button
              type="button"
              onClick={a.onClick}
              aria-label={a.label}
              className="inline-flex items-center gap-1 hover:text-link-hover hover:underline"
            >
              <Icon className="size-4" />
              {a.k && <T k={a.k} />}
            </button>
          </span>
        );
      })}
    </div>
  );
}

const amButton = cva(
  "inline-flex items-center justify-center gap-1.5 border px-3 py-1.5 leading-[1.43] disabled:cursor-not-allowed disabled:opacity-65 focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-1",
  {
    variants: {
      variant: {
        default: "border-button-border bg-button text-button-foreground hover:bg-button-hover",
        danger: "border-danger-border bg-danger text-danger-foreground hover:bg-danger-hover",
        primary: "border-primary bg-primary text-primary-foreground hover:bg-primary/90",
      },
      block: { true: "w-full", false: "" },
    },
    defaultVariants: { variant: "default", block: false },
  },
);

/** AM5 `.btn` (square corners). `danger` is the Compute button. */
export function AmButton({
  className,
  variant,
  block,
  icon,
  k,
  children,
  ...props
}: ComponentProps<"button"> & VariantProps<typeof amButton> & { icon?: IconName; k?: Key }) {
  const Icon = icon ? icons[icon] : null;
  return (
    <button type="button" className={cn(amButton({ variant, block }), className)} {...props}>
      {Icon && <Icon className="size-4" />}
      {k ? <T k={k} /> : children}
    </button>
  );
}

/**
 * AM5 file chooser ("Choose and import data"): an `AmButton` that opens the
 * browser's file picker and hands the chosen files to `onFiles`.
 */
export function FilePicker({
  k,
  accept,
  multiple,
  disabled,
  block,
  onFiles,
}: {
  k: Key;
  /** Extensions or MIME types, e.g. ".tif,.tiff". */
  accept?: string;
  multiple?: boolean;
  disabled?: boolean;
  block?: boolean;
  onFiles: (files: File[]) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <AmButton k={k} block={block} disabled={disabled} onClick={() => input.current?.click()} />
      <input
        ref={input}
        type="file"
        hidden
        accept={accept}
        multiple={multiple}
        onChange={(e) => {
          const files = Array.from(e.target.files ?? []);
          // the same file chosen again must fire a change
          e.target.value = "";
          if (files.length > 0) onFiles(files);
        }}
      />
    </>
  );
}
