import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Modal, Button } from "@metis/ui";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./KillJobModal.module.css";

interface KillJobModalProps {
  open: boolean;
  onClose: () => void;
  jobId: string;
}

export function KillJobModal({ open, onClose, jobId }: KillJobModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => apiClient.killJob(jobId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["job", jobId] });
      queryClient.invalidateQueries({ queryKey: ["jobs"] });
      addToast("Job killed successfully", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to kill job",
        "error",
      );
    },
  });

  const handleConfirm = useCallback(() => {
    mutation.mutate();
  }, [mutation]);

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      onClose();
    }
  }, [mutation.isPending, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title="Kill Job">
      <div className={styles.body}>
        <p className={styles.warning}>
          Are you sure you want to kill this job? This will terminate the
          running job and cannot be undone.
        </p>
        <div className={styles.footer}>
          <Button variant="secondary" size="md" onClick={handleClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            size="md"
            onClick={handleConfirm}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? "Killing..." : "Kill Job"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
