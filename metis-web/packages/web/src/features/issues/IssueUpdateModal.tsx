import { useCallback, useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Modal, Button, Input, Textarea, Select } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { Issue, IssueStatus } from "@metis/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
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

export function IssueUpdateModal({
  open,
  onClose,
  issueId,
  issue,
}: IssueUpdateModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [title, setTitle] = useState(issue.title);
  const [status, setStatus] = useState<IssueStatus>(issue.status);
  const [progress, setProgress] = useState(issue.progress);

  // Reset form when modal opens with fresh issue data
  useEffect(() => {
    if (open) {
      setTitle(issue.title);
      setStatus(issue.status);
      setProgress(issue.progress);
    }
  }, [open, issue.title, issue.status, issue.progress]);

  const mutation = useMutation({
    mutationFn: (params: { title: string; status: IssueStatus; progress: string }) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          title: params.title,
          status: params.status,
          progress: params.progress,
        },
        job_id: null,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      addToast("Issue updated", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to update issue",
        "error",
      );
    },
  });

  const handleSubmit = useCallback(() => {
    mutation.mutate({ title: title.trim(), status, progress });
  }, [title, status, progress, mutation]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      onClose();
    }
  }, [mutation.isPending, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title="Update Issue" className={styles.largeModal}>
      <div className={styles.form} onKeyDown={handleKeyDown}>
        <Input
          label="Title"
          placeholder="Short summary (optional)"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
        />
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
            {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Enter to
            submit
          </span>
          <div className={styles.footerActions}>
            <Button variant="secondary" size="md" onClick={handleClose}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="md"
              onClick={handleSubmit}
              disabled={mutation.isPending}
            >
              {mutation.isPending ? "Saving..." : "Save"}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
