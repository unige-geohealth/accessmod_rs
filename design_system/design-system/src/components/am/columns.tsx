import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * AM5 page grid: controls on the left (Bootstrap col-4), results on the
 * right (col-md-8 col-lg-7). Below md the columns stack.
 * `wide` gives the right column all 8 columns at lg (Data, Project).
 */
export function Columns({ left, right, wide }: { left: ReactNode; right: ReactNode; wide?: boolean }) {
  return (
    <div className="grid grid-cols-12 gap-x-[30px] gap-y-5">
      <div className="col-span-12 md:col-span-4">{left}</div>
      <div className={cn("col-span-12 md:col-span-8", !wide && "lg:col-span-7")}>{right}</div>
    </div>
  );
}
