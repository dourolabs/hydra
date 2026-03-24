import { useCallback, type ReactNode } from "react";
import { Button, Modal } from "@hydra/ui";
import styles from "./DeleteConfirmModal.module.css";

interface DeleteConfirmModalProps {
  open: boolean;
  onClose: () => void;
  entityName: string;
  entityLabel: string;
  onConfirm: () => void;
  isPending: boolean;
  actionLabel?: string;
  pendingLabel?: string;
  description?: ReactNode;
}

export function DeleteConfirmModal({
  open,
  onClose,
  entityName,
  entityLabel,
  onConfirm,
  isPending,
  actionLabel = "Delete",
  pendingLabel = "Deleting...",
  description,
}: DeleteConfirmModalProps) {
  const handleClose = useCallback(() => {
    if (!isPending) {
      onClose();
    }
  }, [isPending, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title={`${actionLabel} ${entityLabel}`}>
      <div className={styles.content}>
        <p className={styles.message}>
          Are you sure you want to {actionLabel.toLowerCase()} this {entityLabel.toLowerCase()}?
          {description && <> {description}</>}
        </p>
        <p className={styles.itemName}>{entityName}</p>
        <div className={styles.actions}>
          <Button
            variant="secondary"
            size="md"
            onClick={handleClose}
            disabled={isPending}
          >
            Cancel
          </Button>
          <Button
            variant="danger"
            size="md"
            onClick={onConfirm}
            disabled={isPending}
          >
            {isPending ? pendingLabel : actionLabel}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
