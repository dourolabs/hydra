import { useCallback, useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Modal, Button, Textarea, Select } from "@hydra/ui";
import type { SelectOption } from "@hydra/ui";
import type { Issue, IssueStatus, IssueVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import largeModalStyles from "../../components/LargeModal.module.css";
import styles from "./IssueUpdateModal.module.css";

const statusOptions: SelectOption[] = [
  { value: "open", label: "Open" },
  { value: "in-progress", label: "In Progress" },
  { value: "closed", label: "Closed" },
  { value: "dropped", label: "Dropped" },
  { value: "rejected", label: "Rejected" },
  { value: "failed", label: "Failed" },
];

interface IssueUpdateModalProps {
  open: boolean;
  onClose: () => void;
  issueId: string;
  issue: Issue;
}

export function IssueUpdateModal({ open, onClose, issueId, issue }: IssueUpdateModalProps) {
  const queryClient = useQueryClient();

  const [status, setStatus] = useState<IssueStatus>(issue.status);
  const [progress, setProgress] = useState(issue.progress);

  // Reset form when modal opens with fresh issue data
  useEffect(() => {
    if (open) {
      setStatus(issue.status);
      setProgress(issue.progress);
    }
  }, [open, issue.status, issue.progress]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    { status: IssueStatus; progress: string },
    unknown,
    { previous?: IssueVersionRecord }
  >({
    mutationFn: (params) =>
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
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: {
            ...previous.issue,
            status: params.status,
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
          onChange={(e) => setStatus(e.target.value as IssueStatus)}
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
