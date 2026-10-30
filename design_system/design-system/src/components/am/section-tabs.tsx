import { type Key, T } from "@/i18n";
import { cn } from "@/lib/utils";
import { type IconName, icons } from "./icons";

export type SectionTab<V extends string> = { value: V; label: Key; icon: IconName };

type SectionTabsProps<V extends string> = {
  tabs: SectionTab<V>[];
  value: V;
  onChange: (v: V) => void;
  label?: string;
  className?: string;
};

/**
 * AM5 inline button group selecting a module (Analysis, Toolbox):
 * `btn btn-default btn-inline` labels in a `data-toggle="buttons"` group.
 * Square corners, pressed look for the active one.
 */
export function SectionTabs<V extends string>({ tabs, value, onChange, label, className }: SectionTabsProps<V>) {
  return (
    <div role="tablist" aria-label={label} className={cn("flex flex-wrap gap-1", className)}>
      {tabs.map((tab) => {
        const Icon = icons[tab.icon];
        const on = tab.value === value;
        return (
          <button
            key={tab.value}
            type="button"
            role="tab"
            aria-selected={on}
            onClick={() => onChange(tab.value)}
            className={cn(
              "inline-flex items-center gap-1 border px-3 py-1.5 text-sm leading-[1.6]",
              "focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-1",
              on
                ? "border-button-border-active bg-button-active text-button-foreground shadow-[inset_0_3px_5px_rgb(0_0_0/0.125)]"
                : "border-button-border bg-button text-button-foreground hover:bg-button-hover",
            )}
          >
            <Icon className="size-4" />
            <T k={tab.label} />
          </button>
        );
      })}
    </div>
  );
}
