import type { ReactNode } from "react";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "#ui/components/ui/dialog";
import { type Key, T } from "#ui/i18n";
import { AmButton } from "./messages";

/**
 * AM5 message modal (`amMsg`), e.g. "Process finished": centred title, the
 * message, then a full-width [OK].
 */
export function MessageDialog({
  open,
  onOpenChange,
  title,
  ok = "analysis.ok",
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: Key;
  ok?: Key;
  children: ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="rounded-none">
        <DialogTitle className="text-center text-3xl font-normal">
          <T k={title} />
        </DialogTitle>
        <DialogDescription asChild>
          <div className="flex flex-col gap-2.5 text-base text-foreground">{children}</div>
        </DialogDescription>
        <AmButton block k={ok} onClick={() => onOpenChange(false)} />
      </DialogContent>
    </Dialog>
  );
}
