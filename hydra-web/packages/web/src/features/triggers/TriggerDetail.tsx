import { useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { Button, Chip } from "@hydra/ui";
import type { TriggerAction, TriggerVersionRecord } from "@hydra/api";
import { hydraIdKind } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { AgoTime } from "../../components/Runtime/Runtime";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import { TriggerEditModal } from "./TriggerEditModal";
import { useTriggerFiringHistory } from "./useTriggerFiringHistory";
import { formatScheduleSummary } from "./scheduleFormat";
import { formatTimestamp } from "../../utils/time";
import styles from "./TriggerDetail.module.css";

interface TriggerDetailProps {
  record: TriggerVersionRecord;
}

export function TriggerDetail({ record }: TriggerDetailProps) {
  const { trigger, trigger_id } = record;
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { addToast } = useToast();

  const deleteMutation = useMutation({
    mutationFn: () => apiClient.deleteTrigger(trigger_id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["triggers"] });
      addToast("Trigger deleted", "success");
      navigate("/triggers");
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete trigger",
        "error",
      );
    },
  });

  return (
    <div className={styles.detail}>
      <div className={styles.head}>
        <div className={styles.headMain}>
          <span className={styles.eyebrow}>TRIGGER</span>
          <h1 className={styles.title}>{trigger_id}</h1>
          <div className={styles.headMeta}>
            <Chip tone={trigger.enabled ? "acc" : "muted"}>
              {trigger.enabled ? "enabled" : "disabled"}
            </Chip>
            <span className={styles.scheduleSummary}>
              {formatScheduleSummary(trigger.schedule)}
            </span>
          </div>
        </div>
        <div className={styles.headActions}>
          <Button variant="secondary" size="sm" onClick={() => setEditOpen(true)}>
            Edit
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setDeleteOpen(true)}>
            Delete
          </Button>
        </div>
      </div>

      <div className={styles.grid}>
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Schedule</h2>
          <ScheduleSummary record={record} />
        </section>

        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Actions</h2>
          {trigger.actions.length === 0 ? (
            <p className={styles.empty}>No actions configured.</p>
          ) : (
            <div className={styles.actionsList}>
              {trigger.actions.map((action, idx) => (
                <ActionPanel key={idx} index={idx} action={action} />
              ))}
            </div>
          )}
        </section>

        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Firing history</h2>
          <FiringHistory triggerId={trigger_id} />
        </section>
      </div>

      <TriggerEditModal
        open={editOpen}
        onClose={() => setEditOpen(false)}
        record={record}
      />

      {deleteOpen && (
        <DeleteConfirmModal
          open={deleteOpen}
          onClose={() => setDeleteOpen(false)}
          entityName={trigger_id}
          entityLabel="Trigger"
          onConfirm={() => deleteMutation.mutate()}
          isPending={deleteMutation.isPending}
        />
      )}
    </div>
  );
}

function ScheduleSummary({ record }: { record: TriggerVersionRecord }) {
  const { trigger } = record;
  return (
    <dl className={styles.dl}>
      <DataRow label="Schedule">
        <code className={styles.code}>
          {formatScheduleSummary(trigger.schedule)}
        </code>
      </DataRow>
      <DataRow label="Creator">{trigger.creator}</DataRow>
      <DataRow label="Last fired">
        {trigger.last_fired_at ? (
          <>
            <AgoTime iso={trigger.last_fired_at} />
            <span className={styles.absoluteTs}>
              {" · "}
              {formatTimestamp(trigger.last_fired_at)}
            </span>
          </>
        ) : (
          <span className={styles.muted}>never</span>
        )}
      </DataRow>
      <DataRow label="Version">{String(record.version)}</DataRow>
      <DataRow label="Updated">{formatTimestamp(record.timestamp)}</DataRow>
    </dl>
  );
}

function ActionPanel({ index, action }: { index: number; action: TriggerAction }) {
  const ci = action.CreateIssue;
  return (
    <div className={styles.actionCard}>
      <div className={styles.actionCardHead}>
        <span className={styles.actionCardLabel}>
          Action {index + 1} · CreateIssue
        </span>
        <span className={styles.actionTypeChip}>{ci.type}</span>
      </div>
      <dl className={styles.dl}>
        <DataRow label="Title">
          <code className={styles.code}>{ci.title}</code>
        </DataRow>
        <DataRow label="Description">
          <code className={styles.codeBlock}>{ci.description}</code>
        </DataRow>
        {ci.assignee && (
          <DataRow label="Assignee">
            <code className={styles.code}>{ci.assignee}</code>
          </DataRow>
        )}
        {ci.status && <DataRow label="Status">{ci.status}</DataRow>}
        {ci.session_settings?.repo_name && (
          <DataRow label="Repository">{ci.session_settings.repo_name}</DataRow>
        )}
      </dl>
    </div>
  );
}

function FiringHistory({ triggerId }: { triggerId: string }) {
  const { data, isLoading, error } = useTriggerFiringHistory(triggerId);

  if (isLoading) return <p className={styles.muted}>Loading…</p>;
  if (error) {
    return (
      <p className={styles.error}>
        Failed to load firing history: {(error as Error).message}
      </p>
    );
  }
  if (!data || data.length === 0) {
    return (
      <p className={styles.muted}>
        No fires recorded yet. The worker writes a `created` edge to this
        trigger every time it fires.
      </p>
    );
  }
  return (
    <div className={styles.firingList} data-testid="trigger-firing-history">
      {data.map((rel) => (
        <div
          key={`${rel.target_id}-${rel.created_at}`}
          className={styles.firingRow}
          data-testid={`trigger-firing-row-${rel.target_id}`}
        >
          <TargetLink targetId={rel.target_id} />
          <span className={styles.firingTime}>
            <AgoTime iso={rel.created_at} />
            <span className={styles.absoluteTs}>
              {" · "}
              {formatTimestamp(rel.created_at)}
            </span>
          </span>
        </div>
      ))}
    </div>
  );
}

function TargetLink({ targetId }: { targetId: string }) {
  const kind = hydraIdKind(targetId);
  const to = pathForTarget(kind, targetId);
  if (!to) {
    return <code className={styles.code}>{targetId}</code>;
  }
  return (
    <Link to={to} className={styles.firingLink}>
      <code className={styles.code}>{targetId}</code>
    </Link>
  );
}

function pathForTarget(
  kind: ReturnType<typeof hydraIdKind>,
  id: string,
): string | null {
  switch (kind) {
    case "issue":
      return `/issues/${id}`;
    case "patch":
      return `/patches/${id}`;
    case "document":
      return `/documents/${id}`;
    case "session":
      return `/sessions/${id}`;
    case "conversation":
      return `/chat/${id}`;
    default:
      return null;
  }
}

function DataRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className={styles.dlRow}>
      <dt className={styles.dlLabel}>{label}</dt>
      <dd className={styles.dlValue}>{children}</dd>
    </div>
  );
}
