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
}

export function IssueStatusPicker({
  issueId,
  issue,
  hideLabel,
}: IssueStatusPickerProps) {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const { data: projectStatuses } = useProjectStatuses(issue.project_id);

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
          issue: { ...previous.issue, status: optimisticStatus },
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
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });

  const choose = (next: StatusKey) => {
    setOpen(false);
    if (next === current) return;
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
      value={<StatusChip status={issue.status} />}
    >
      {statusEntries.length === 0 ? (
        <div className={styles.popEmpty}>No statuses</div>
      ) : (
        statusEntries.map((s) => (
          <PickerRow
            key={s.key}
            active={current === s.key}
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
