import { useCallback, useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Modal, Button, Textarea } from "@hydra/ui";
import type { IssueVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useFormModal } from "../../hooks/useFormModal";
import largeModalStyles from "../../components/LargeModal.module.css";
import styles from "./FeedbackModal.module.css";

interface FeedbackModalProps {
  open: boolean;
  onClose: () => void;
  issueId: string;
}

export function FeedbackModal({ open, onClose, issueId }: FeedbackModalProps) {
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState("");

  useEffect(() => {
    if (open) {
      setFeedback("");
    }
  }, [open]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<string, unknown>({
    mutationFn: (text) => apiClient.submitFeedback(issueId, text),
    invalidateKeys: [["issue", issueId], ["issues"]],
    successMessage: "Feedback submitted",
    onSuccess: () => {
      onClose();
    },
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
    onError: (_err, _variables, context) => {
      const ctx = context as { previous?: IssueVersionRecord } | undefined;
      if (ctx?.previous) {
        queryClient.setQueryData(["issue", issueId], ctx.previous);
      }
    },
  });

  const handleSubmit = useCallback(() => {
    const trimmed = feedback.trim();
    if (!trimmed) return;
    mutation.mutate(trimmed);
  }, [feedback, mutation]);

  const noop = useCallback(() => {}, []);

  return (
    <Modal
      open={open}
      onClose={() => handleClose(noop, onClose)}
      title="Give Feedback"
      className={largeModalStyles.largeModal}
    >
      <div className={styles.form} onKeyDown={(e) => handleKeyDown(e, handleSubmit)}>
        <Textarea
          label="Feedback"
          placeholder="Describe what you'd like the agent to change..."
          value={feedback}
          onChange={(e) => setFeedback(e.target.value)}
          className={styles.feedbackTextarea}
        />
        <div className={styles.footer}>
          <span className={styles.hint}>
            {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+Enter to submit
          </span>
          <div className={styles.footerActions}>
            <Button variant="secondary" size="md" onClick={() => handleClose(noop, onClose)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="md"
              onClick={handleSubmit}
              disabled={isPending || !feedback.trim()}
            >
              {isPending ? "Submitting..." : "Submit Feedback"}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
