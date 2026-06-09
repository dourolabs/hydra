import { useCallback, useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Modal, Button, Textarea, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type { Issue, IssueVersionRecord, StatusKey } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import { useProjectStatuses } from "../projects/useProjects";
import largeModalStyles from "../../components/LargeModal.module.css";
import styles from "./IssueUpdateModal.module.css";

interface IssueUpdateModalProps {
  open: boolean;
  onClose: () => void;
  issueId: string;
  issue: Issue;
}

export function IssueUpdateModal({ open, onClose, issueId, issue }: IssueUpdateModalProps) {
  const queryClient = useQueryClient();

  const [status, setStatus] = useState<StatusKey>(issue.status.key);
  const [progress, setProgress] = useState(issue.progress);

  // Pull status options from the issue's project. The hook caches per
  // project for the session via React Query.
  const { data: projectStatuses } = useProjectStatuses(issue.project_id);
  const statusOptions: SelectOption[] = useMemo(() => {
    const list = projectStatuses?.statuses ?? [];
    if (list.length === 0) {
      // Until the fetch resolves, keep the current status as the sole option so
      // the Select doesn't flip to an unrelated default and clobber state.
      return [{ value: issue.status.key, label: issue.status.label }];
    }
    return list.map((s) => ({ value: s.key, label: s.label }));
  }, [projectStatuses, issue.status]);

  // Reset form when modal opens with fresh issue data
  useEffect(() => {
    if (open) {
      setStatus(issue.status.key);
      setProgress(issue.progress);
    }
  }, [open, issue.status, issue.progress]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    { status: StatusKey; progress: string },
    unknown,
    { previous?: IssueVersionRecord }
  >({
    mutationFn: (params) =>
      // The wire `IssueInput` carries the bare `StatusKey`; the spread
      // of the response-shaped `issue` is followed by `status` /
      // `progress` overrides so the resulting object matches the
      // request type.
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: params.status,
          progress: params.progress,
        },
        session_id: null,
      }),
    invalidateKeys: [["issue", issueId], ["issues"]],
    successMessage: "Issue updated",
    onSuccess: () => {
      onClose();
    },
    onMutate: async (params) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>(["issue", issueId]);
      if (previous) {
        // Project the new key onto the cached `StatusDefinition` so the
        // optimistic update keeps the response shape. Display props
        // (label, color, flags) snap to the canonical resolution on the
        // next server response.
        const optimisticStatus =
          projectStatuses?.statuses.find((s) => s.key === params.status) ?? {
            ...previous.issue.status,
            key: params.status,
          };
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: {
            ...previous.issue,
            status: optimisticStatus,
            progress: params.progress,
          },
        });
      }
      return { previous };
    },
    onError: (_err, _variables, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
    },
  });

  const handleSubmit = useCallback(() => {
    mutation.mutate({ status, progress });
  }, [status, progress, mutation]);

  return (
    <Modal
      open={open}
      onClose={() => handleClose(onClose)}
      title="Update Issue"
      className={largeModalStyles.largeModal}
    >
      <div className={styles.form} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
        <Select
          label="Status"
          options={statusOptions}
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        />
        <div className={styles.progressWrapper}>
          <Textarea
            label="Progress"
            placeholder="Describe current progress..."
            value={progress}
            onChange={(e) => setProgress(e.target.value)}
            className={styles.progressTextarea}
          />
        </div>
        <div className={styles.footer}>
          <span className={styles.hint}>
            {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Enter to submit
          </span>
          <div className={styles.footerActions}>
            <Button variant="secondary" size="md" onClick={() => handleClose(onClose)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="md"
              onClick={handleSubmit}
              disabled={isPending}
            >
              {isPending ? "Saving..." : "Save"}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
