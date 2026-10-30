import { Accordion as AccordionPrimitive } from "radix-ui";
import type { ReactNode } from "react";
import { type Key, T } from "@/i18n";
import { cn } from "@/lib/utils";
import { type IconName, icons } from "./icons";

export type PanelItem = {
  value: string;
  title: Key | ReactNode;
  icon?: IconName;
  content: ReactNode;
};

type PanelAccordionProps = {
  items: PanelItem[];
  /** Open item. AM5 groups are exclusive: opening one closes the others. */
  value?: string;
  defaultValue?: string;
  onValueChange?: (v: string) => void;
  className?: string;
};

/**
 * AM5 `amAccordionGroup` (Bootstrap 3 `.panel-group`): grey headings,
 * square panels 5px apart, one item open at a time, all can be closed.
 */
export function PanelAccordion({ items, value, defaultValue, onValueChange, className }: PanelAccordionProps) {
  return (
    <AccordionPrimitive.Root
      type="single"
      collapsible
      value={value}
      defaultValue={defaultValue}
      onValueChange={onValueChange}
      className={cn("flex flex-col gap-[5px]", className)}
    >
      {items.map((item) => {
        const Icon = item.icon ? icons[item.icon] : null;
        return (
          <AccordionPrimitive.Item
            key={item.value}
            value={item.value}
            className="border border-panel-border bg-panel shadow-[0_1px_1px_rgb(0_0_0/0.05)]"
          >
            <AccordionPrimitive.Header asChild>
              <h4>
                <AccordionPrimitive.Trigger
                  className={cn(
                    "flex w-full items-center gap-1 border-panel-border bg-panel-heading px-[15px] py-2.5 text-left text-base text-panel-heading-foreground",
                    "data-[state=open]:border-b focus-visible:outline-2 focus-visible:outline-ring focus-visible:-outline-offset-2",
                  )}
                >
                  {Icon && <Icon className="size-4 shrink-0" />}
                  {typeof item.title === "string" ? <T k={item.title as Key} /> : item.title}
                </AccordionPrimitive.Trigger>
              </h4>
            </AccordionPrimitive.Header>
            <AccordionPrimitive.Content className="overflow-hidden data-[state=closed]:animate-accordion-up data-[state=open]:animate-accordion-down">
              <div className="p-[15px]">{item.content}</div>
            </AccordionPrimitive.Content>
          </AccordionPrimitive.Item>
        );
      })}
    </AccordionPrimitive.Root>
  );
}
