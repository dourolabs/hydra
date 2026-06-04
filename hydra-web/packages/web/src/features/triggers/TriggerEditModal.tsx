import { useCallback, useState } from "react";
import { Button, Modal } from "@hydra/ui";
import type {
  TriggerVersionRecord,
  UpsertTriggerRequest,
  UpsertTriggerResponse,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { TriggerForm } from "./TriggerForm";
import {
  buildUpsertRequest,
  initialDraftFromExisting,
  type TriggerDraft,
} from "./triggerDraft";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";
import styles from "./TriggersSection.module.css";

interface TriggerEditModalProps {
  open: boolean;
  onClose: () => void;
  record: TriggerVersionRecord;
}

export function TriggerEditModal({
  open,
  onClose,
  record,
}: TriggerEditModalProps) {
  const [draft, setDraft] = useState<TriggerDraft>(() =>
    initialDraftFromExisting(
      record.trigger.schedule,
      record.trigger.enabled,
      record.trigger.actions,
    ),
  );

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    UpsertTriggerRequest,
    UpsertTriggerResponse
  >({
    mutationFn: (params) =>
      apiClient.updateTrigger(record.trigger_id, params),
    invalidateKeys: [
      ["triggers"],
      ["trigger", record.trigger_id],
    ],
    successMessage: "Trigger updated",
    onSuccess: () => {
      onClose();
    },
  });

  const handleSubmit = useCallback(() => {
    const req = buildUpsertRequest(draft, record.trigger.creator);
    if (!req) return;
    mutation.mutate(req);
  }, [draft, record.trigger.creator, mutation]);

  const isValid = buildUpsertRequest(draft, record.trigger.creator) != null;

  return (
    <Modal
      open={open}
      onClose={() => handleClose(onClose)}
      title={`Edit ${record.trigger_id}`}
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
            onClick={() => handleClose(onClose)}
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
            {isPending ? "Saving..." : "Save Changes"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
