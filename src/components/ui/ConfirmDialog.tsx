import type { ReactNode } from "react";
import { Dialog } from "./Dialog";

type ConfirmDialogProps = {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: ReactNode;
  description?: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
};

export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  description,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  destructive = false,
}: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      description={description}
      width={420}
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose}>
            {cancelLabel}
          </button>
          <button
            type="button"
            className={`primary-action primary-solid${
              destructive ? " primary-destructive" : ""
            }`}
            onClick={() => {
              onConfirm();
              onClose();
            }}
          >
            {confirmLabel}
          </button>
        </>
      }
    >
      <div />
    </Dialog>
  );
}
