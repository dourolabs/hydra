import { useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Chip } from "@hydra/ui";
import type { TriggerVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useTriggers } from "./useTriggers";
import { useAuth } from "../auth/useAuth";
import { actorDisplayName } from "../../api/auth";
import { LoadingState } from "../../components/LoadingState/LoadingState";
import { ErrorState } from "../../components/ErrorState/ErrorState";
import { EmptyState } from "../../components/EmptyState/EmptyState";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import { useToast } from "../toast/useToast";
import { TriggerCreateModal } from "./TriggerCreateModal";
import { formatScheduleSummary } from "./scheduleFormat";
import { AgoTime } from "../../components/Runtime/Runtime";
import styles from "./TriggersSection.module.css";

interface TriggersSectionProps {
  createOpen: boolean;
  onCreateOpenChange: (open: boolean) => void;
}

export function TriggersSection({
  createOpen,
  onCreateOpenChange,
}: TriggersSectionProps) {
  const { data: triggers, isLoading, error, refetch } = useTriggers();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const creator = user ? actorDisplayName(user.actor) : "";

  const [deleteTarget, setDeleteTarget] =
    useState<TriggerVersionRecord | null>(null);

  const deleteMutation = useMutation({
    mutationFn: (triggerId: string) => apiClient.deleteTrigger(triggerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["triggers"] });
      addToast("Trigger deleted", "success");
      setDeleteTarget(null);
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete trigger",
        "error",
      );
    },
  });

  return (
    <>
      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load triggers: ${(error as Error).message}`}
          onRetry={() => refetch()}
        />
      )}

      {triggers && triggers.length === 0 && (
        <EmptyState message="No triggers configured." />
      )}

      {triggers && triggers.length > 0 && (
        <div className={styles.cards} data-testid="triggers-list">
          {triggers.map((record) => (
            <TriggerCard
              key={record.trigger_id}
              record={record}
              onDelete={() => setDeleteTarget(record)}
            />
          ))}
        </div>
      )}

      <TriggerCreateModal
        open={createOpen}
        onClose={() => onCreateOpenChange(false)}
        creator={creator}
      />

      {deleteTarget && (
        <DeleteConfirmModal
          open={!!deleteTarget}
          onClose={() => setDeleteTarget(null)}
          entityName={deleteTarget.trigger_id}
          entityLabel="Trigger"
          onConfirm={() => deleteMutation.mutate(deleteTarget.trigger_id)}
          isPending={deleteMutation.isPending}
        />
      )}
    </>
  );
}

interface TriggerCardProps {
  record: TriggerVersionRecord;
  onDelete: () => void;
}

function TriggerCard({ record, onDelete }: TriggerCardProps) {
  const { trigger, trigger_id } = record;
  const actionCount = trigger.actions.length;
  return (
    <div
      className={styles.card}
      data-testid={`triggers-list-card-${trigger_id}`}
    >
      <div className={styles.cardHead}>
        <Link to={`/triggers/${trigger_id}`} className={styles.cardLink}>
          {trigger_id}
        </Link>
        <span className={styles.cardHeadSpacer} />
        <Chip tone={trigger.enabled ? "acc" : "muted"}>
          {trigger.enabled ? "enabled" : "disabled"}
        </Chip>
      </div>

      <div className={styles.schedulePreview} title={formatScheduleSummary(trigger.schedule)}>
        {formatScheduleSummary(trigger.schedule)}
      </div>

      <div className={styles.metaRow}>
        <span className={styles.metaChip}>
          <span className={styles.metaChipKey}>actions</span>
          {actionCount}
        </span>
        <span className={styles.metaChip}>
          <span className={styles.metaChipKey}>last fired</span>
          {trigger.last_fired_at ? (
            <AgoTime iso={trigger.last_fired_at} />
          ) : (
            "never"
          )}
        </span>
        <span className={styles.metaChip}>
          <span className={styles.metaChipKey}>creator</span>
          {trigger.creator}
        </span>
      </div>

      <div className={styles.cardFoot}>
        <span className={styles.cardFootSpacer} />
        <Link to={`/triggers/${trigger_id}`}>
          <Button variant="ghost" size="sm">
            Open
          </Button>
        </Link>
        <Button variant="ghost" size="sm" onClick={onDelete}>
          Delete
        </Button>
      </div>
    </div>
  );
}
