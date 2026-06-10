import { ConfirmModal, type ConfirmModalProps } from "@hydra/ui";

type DeleteConfirmModalProps = ConfirmModalProps;

export function DeleteConfirmModal({
  actionLabel = "Delete",
  pendingLabel = "Deleting...",
  ...rest
}: DeleteConfirmModalProps) {
  return (
    <ConfirmModal
      {...rest}
      actionLabel={actionLabel}
      pendingLabel={pendingLabel}
    />
  );
}
