import { Tabs as TabsPrimitive } from "radix-ui";
import type { ReactNode } from "react";
import { type Key, T } from "@/i18n";
import { cn } from "@/lib/utils";

export type PageTab<V extends string> = { value: V; label: Key; content: ReactNode };

/**
 * AM5 `tabsetPanel` (Bootstrap `.nav-tabs`): link-coloured labels on a
 * bottom rule, the active tab is a bordered "folder" tab. Used for steps
 * inside one tool (Merge land cover: Land cover, Roads, Barriers, Merge).
 * Not for switching modules: use SectionTabs.
 */
export function PageTabs<V extends string>({
  tabs,
  value,
  defaultValue,
  onValueChange,
}: {
  tabs: PageTab<V>[];
  value?: V;
  defaultValue?: V;
  onValueChange?: (v: V) => void;
}) {
  return (
    <TabsPrimitive.Root
      value={value}
      defaultValue={defaultValue ?? tabs[0]?.value}
      onValueChange={(v) => onValueChange?.(v as V)}
    >
      <TabsPrimitive.List className="flex border-b border-panel-border">
        {tabs.map((tab) => (
          <TabsPrimitive.Trigger
            key={tab.value}
            value={tab.value}
            className={cn(
              "-mb-px rounded-t-[var(--radius)] border border-transparent px-[15px] py-2.5 text-link",
              "hover:border-separator hover:bg-muted",
              "data-[state=active]:border-panel-border data-[state=active]:border-b-background data-[state=active]:bg-background data-[state=active]:text-foreground",
              "focus-visible:outline-2 focus-visible:outline-ring",
            )}
          >
            <T k={tab.label} />
          </TabsPrimitive.Trigger>
        ))}
      </TabsPrimitive.List>
      {tabs.map((tab) => (
        <TabsPrimitive.Content key={tab.value} value={tab.value} className="pt-[30px]">
          {tab.content}
        </TabsPrimitive.Content>
      ))}
    </TabsPrimitive.Root>
  );
}
