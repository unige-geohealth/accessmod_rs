import type { ReactNode } from "react";
import { type Key, T } from "#ui/i18n";
import { cn } from "#ui/lib/utils";
import { type IconName, icons } from "./icons";

/**
 * AM5 `amCenterTitle`: centered heading, subtitle, then a rule.
 * Used as the head of every content column ("Available data", "Travel scenario").
 */
export function CenterTitle({
  title,
  sub,
  icon,
  level = 2,
  className,
}: {
  title: Key;
  sub?: Key;
  icon?: IconName;
  level?: 2 | 3;
  className?: string;
}) {
  const Icon = icon ? icons[icon] : null;
  const H = level === 2 ? "h2" : "h3";
  return (
    <div className={cn("text-center", className)}>
      <H
        className={cn(
          "inline-flex items-center gap-1.5 font-normal",
          level === 2 ? "mt-[50px] mb-2.5 text-[30px] leading-tight" : "mt-2.5 mb-2.5 text-2xl leading-tight",
        )}
      >
        {Icon && <Icon className={level === 2 ? "size-7" : "size-6"} />}
        <T k={title} />
      </H>
      {sub && (
        <p className="mb-2.5">
          <T k={sub} />
        </p>
      )}
      <hr className="my-5 border-separator" />
    </div>
  );
}

/**
 * AM5 left-column card (`.well`): grey box holding a centered icon title
 * and a form, e.g. "Data input", "Analysis settings", "Validation".
 */
export function SectionCard({
  title,
  sub,
  icon,
  children,
  className,
}: {
  title: Key;
  sub?: Key;
  icon?: IconName;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cn(
        "mb-5 border border-well-border bg-well p-[19px] shadow-[inset_0_1px_1px_rgb(0_0_0/0.05)]",
        className,
      )}
    >
      <CenterTitle title={title} sub={sub} icon={icon} level={3} />
      <div className="flex flex-col gap-[15px]">{children}</div>
    </section>
  );
}

/** Content column subheading ("Travel scenario to be processed"). */
export function SubTitle({ k, children }: { k?: Key; children?: ReactNode }) {
  return <h4 className="mt-2.5 mb-2.5 text-lg">{k ? <T k={k} /> : children}</h4>;
}
