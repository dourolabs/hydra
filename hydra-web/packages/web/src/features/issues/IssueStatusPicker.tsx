import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Picker, PickerRow } from "@hydra/ui";
import type { Issue, IssueVersionRecord, StatusKey } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useProjectStatuses } from "../projects/useProjects";
import { useToast } from "../toast/useToast";
import { StatusChip } from "../projects/StatusChip";
import styles from "./IssueStatusPicker.module.css";

interface IssueStatusPickerProps {
  issueId: string;
  issue: Issue;
  /**
   * Hide the visual "Status" caption above the trigger pill. The label
   * is still wired through to the trigger's `aria-label`.
   */
  hideLabel?: boolean;
  /**
   * When set, the picker switches into "pending project" mode:
   * — statuses are sourced from the pending project,
   * — the trigger shows a "Select a status…" placeholder rather than the
   *   persisted status chip,
   * — picking a status fires a SINGLE `updateIssue` mutation that
   *   commits both the new `project_id` and the new `status` atomically.
   */
  pendingProjectId?: string | null;
  /**
   * Called after the atomic status+project mutation succeeds in pending
   * mode, so the parent can clear its pending state.
   */
  onPendingResolved?: () => void;
}

export function IssueStatusPicker({
  issueId,
  issue,
  hideLabel,
  pendingProjectId,
  onPendingResolved,
}: IssueStatusPickerProps) {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();
  const { addToast } = useToast();

  const pending = pendingProjectId != null;
  const effectiveProjectId = pendingProjectId ?? issue.project_id;
  const { data: projectStatuses } = useProjectStatuses(effectiveProjectId);

  const statusEntries = projectStatuses?.statuses ?? [];
  const current = issue.status.key;

  const mutation = useMutation<
    unknown,
    Error,
    StatusKey,
    { previous?: IssueVersionRecord }
  >({
    mutationFn: (statusKey) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: statusKey,
          project_id: effectiveProjectId,
        },
        session_id: null,
      }),
    onMutate: async (statusKey) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previous) {
        const optimisticStatus =
          statusEntries.find((s) => s.key === statusKey) ?? {
            ...previous.issue.status,
            key: statusKey,
          };
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: {
            ...previous.issue,
            status: optimisticStatus,
            project_id: effectiveProjectId,
          },
        });
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(err.message || "Failed to update status", "error");
    },
    onSuccess: () => {
      if (pending) {
        onPendingResolved?.();
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });

  const choose = (next: StatusKey) => {
    setOpen(false);
    // In pending mode any pick fires the atomic mutation — even if
    // `next === current` (the new project may genuinely declare the same
    // key, but we still need the project_id field to be persisted).
    if (!pending && next === current) return;
    mutation.mutate(next);
  };

  return (
    <Picker
      label="Status"
      hideLabel={hideLabel}
      open={open}
      onToggle={() => setOpen((v) => !v)}
      wide
      data-testid="issue-status-picker"
      value={
        pending ? (
          <span className={styles.pillEmpty}>Select a status…</span>
        ) : (
          <StatusChip status={issue.status} />
        )
      }
    >
      {statusEntries.length === 0 ? (
        <div className={styles.popEmpty}>No statuses</div>
      ) : (
        statusEntries.map((s) => (
          <PickerRow
            key={s.key}
            active={!pending && current === s.key}
            onClick={() => choose(s.key)}
            data-testid={`issue-status-option-${s.key}`}
          >
            <StatusChip status={s} />
            <span className={styles.popSpacer} />
          </PickerRow>
        ))
      )}
    </Picker>
  );
}
