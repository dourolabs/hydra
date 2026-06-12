import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Picker, PickerRow } from "@hydra/ui";
import type { Issue, IssueVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useProjects } from "../projects/useProjects";
import { useToast } from "../toast/useToast";
import styles from "./IssueProjectPicker.module.css";

interface IssueProjectPickerProps {
  issueId: string;
  issue: Issue;
  /**
   * Hide the visual "Project" caption above the trigger pill. The label
   * is still wired through to the trigger's `aria-label`.
   */
  hideLabel?: boolean;
  /**
   * When provided, the picker switches into "coordinated" mode: the
   * trigger pill reflects `pendingProjectId ?? issue.project_id`, and
   * picking a row calls `onPendingChange(nextId)` instead of firing the
   * mutation. The parent is responsible for clearing the pending state
   * by passing `null` when the user re-picks the persisted project; this
   * picker forwards the raw selection verbatim.
   */
  pendingProjectId?: string | null;
  /**
   * Set together with `pendingProjectId`. Called whenever the user picks
   * a row in coordinated mode; `null` means "no pending change".
   */
  onPendingChange?: (projectId: string | null) => void;
}

export function IssueProjectPicker({
  issueId,
  issue,
  hideLabel,
  pendingProjectId,
  onPendingChange,
}: IssueProjectPickerProps) {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();
  const { addToast } = useToast();
  const { data: projects } = useProjects();

  const coordinated = onPendingChange !== undefined;
  const activeProjectId = pendingProjectId ?? issue.project_id;
  const selectedProject = projects?.find(
    (p) => p.project_id === activeProjectId,
  );

  const mutation = useMutation<
    unknown,
    Error,
    string,
    { previous?: IssueVersionRecord }
  >({
    mutationFn: (projectId) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: issue.status.key,
          project_id: projectId,
        },
        session_id: null,
      }),
    onMutate: async (projectId) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: { ...previous.issue, project_id: projectId },
        });
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(err.message || "Failed to update project", "error");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });

  const choose = (next: string) => {
    setOpen(false);
    if (next === activeProjectId) return;
    if (coordinated) {
      onPendingChange(next === issue.project_id ? null : next);
      return;
    }
    mutation.mutate(next);
  };

  return (
    <Picker
      label="Project"
      hideLabel={hideLabel}
      open={open}
      onToggle={() => setOpen((v) => !v)}
      wide
      data-testid="issue-project-picker"
      value={
        selectedProject ? (
          <span className={styles.pillContent}>
            <code className={styles.pillCode}>{selectedProject.project.key}</code>
          </span>
        ) : (
          <span className={styles.pillEmpty}>No project</span>
        )
      }
    >
      {(projects ?? []).map((p) => (
        <PickerRow
          key={p.project_id}
          active={activeProjectId === p.project_id}
          onClick={() => choose(p.project_id)}
          data-testid={`issue-project-option-${p.project.key}`}
        >
          <code className={styles.popCode}>{p.project.key}</code>
          <span className={styles.popSub}>{p.project.name}</span>
          <span className={styles.popSpacer} />
        </PickerRow>
      ))}
    </Picker>
  );
}
