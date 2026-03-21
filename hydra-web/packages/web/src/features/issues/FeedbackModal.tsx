import { useCallback, useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Modal, Button, Textarea } from "@hydra/ui";
import type { IssueVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./FeedbackModal.module.css";

interface FeedbackModalProps {
  open: boolean;
  onClose: () => void;
  issueId: string;
}

export function FeedbackModal({ open, onClose, issueId }: FeedbackModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState("");

  useEffect(() => {
    if (open) {
      setFeedback("");
    }
  }, [open]);

  const mutation = useMutation({
    mutationFn: (text: string) => apiClient.submitFeedback(issueId, text),
    onMutate: async (text) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>(["issue", issueId]);
      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: {
            ...previous.issue,
            feedback: text,
          },
        });
      }
      return { previous };
    },
    onSuccess: () => {
      addToast("Feedback submitted", "success");
      onClose();
    },
    onError: (err, _variables, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to submit feedback",
        "error",
      );
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });

  const handleSubmit = useCallback(() => {
    const trimmed = feedback.trim();
    if (!trimmed) return;
    mutation.mutate(trimmed);
  }, [feedback, mutation]);

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
    <Modal open={open} onClose={handleClose} title="Give Feedback">
      <div className={styles.form} onKeyDown={handleKeyDown}>
        <Textarea
          label="Feedback"
          placeholder="Describe what you'd like the agent to change..."
          value={feedback}
          onChange={(e) => setFeedback(e.target.value)}
          className={styles.feedbackTextarea}
        />
        <div className={styles.footer}>
          <span className={styles.hint}>
            {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+Enter to
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
              disabled={mutation.isPending || !feedback.trim()}
            >
              {mutation.isPending ? "Submitting..." : "Submit Feedback"}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
