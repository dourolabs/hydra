import { useCallback, useState } from "react";
import { Button, Modal } from "@hydra/ui";
import type { UpsertTriggerRequest, UpsertTriggerResponse } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { TriggerForm } from "./TriggerForm";
import {
  buildUpsertRequest,
  emptyTriggerDraft,
  type TriggerDraft,
} from "./triggerDraft";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";
import styles from "./TriggersSection.module.css";

interface TriggerCreateModalProps {
  open: boolean;
  onClose: () => void;
  creator: string;
}

export function TriggerCreateModal({
  open,
  onClose,
  creator,
}: TriggerCreateModalProps) {
  const [draft, setDraft] = useState<TriggerDraft>(emptyTriggerDraft);

  const resetForm = useCallback(() => {
    setDraft(emptyTriggerDraft());
  }, []);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    UpsertTriggerRequest,
    UpsertTriggerResponse
  >({
    mutationFn: (params) => apiClient.createTrigger(params),
    invalidateKeys: [["triggers"]],
    successMessage: (data) => `Trigger ${data.trigger_id} created`,
    onSuccess: () => {
      resetForm();
      onClose();
    },
  });

  const handleSubmit = useCallback(() => {
    const req = buildUpsertRequest(draft, creator);
    if (!req) return;
    mutation.mutate(req);
  }, [draft, creator, mutation]);

  const isValid = buildUpsertRequest(draft, creator) != null;

  return (
    <Modal
      open={open}
      onClose={() => handleClose(onClose, resetForm)}
      title="Add Trigger"
    >
      <div
        className={sharedStyles.formFields}
        onKeyDown={(e) => handleKeyDown(e, handleSubmit)}
      >
        <TriggerForm draft={draft} onChange={setDraft} />
        {!isValid && (
          <p className={styles.formError}>
            Fill in schedule + at least one action with title and description.
          </p>
        )}
        <div className={sharedStyles.formActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={() => handleClose(onClose, resetForm)}
            disabled={isPending}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleSubmit}
            disabled={!isValid || isPending}
          >
            {isPending ? "Creating..." : "Add Trigger"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
