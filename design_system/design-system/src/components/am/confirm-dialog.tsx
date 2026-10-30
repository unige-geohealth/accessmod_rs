import type { ReactNode } from "react";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "#ui/components/ui/dialog";
import { type Key, T } from "#ui/i18n";
import { AmButton } from "./messages";

/**
 * AM5 confirmation modal (`amUpdateModal`): "Confirmation" title, the
 * consequence, then [Cancel] and the (danger) action.
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title = "srv_data_modal_confirmation_1",
  confirm,
  cancel = "data_delete_cancel",
  onConfirm,
  danger = true,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title?: Key;
  confirm: Key;
  cancel?: Key;
  onConfirm: () => void;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showCloseButton={false} className="rounded-none">
        <DialogTitle className="text-lg font-medium">
          <T k={title} />
        </DialogTitle>
        <DialogDescription asChild>
          <div className="text-foreground">{children}</div>
        </DialogDescription>
        <DialogFooter>
          <AmButton k={cancel} onClick={() => onOpenChange(false)} />
          <AmButton
            k={confirm}
            variant={danger ? "danger" : "primary"}
            onClick={() => {
              onOpenChange(false);
              onConfirm();
            }}
          />
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
