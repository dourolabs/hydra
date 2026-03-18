import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { SessionVersionRecord } from "@hydra/api";
import { Modal, Button } from "@hydra/ui";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./KillSessionModal.module.css";

interface KillSessionModalProps {
  open: boolean;
  onClose: () => void;
  onKillSuccess?: () => void;
  sessionId: string;
}

export function KillSessionModal({ open, onClose, onKillSuccess, sessionId }: KillSessionModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => apiClient.killSession(sessionId),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["session", sessionId] });
      const previous = queryClient.getQueryData<SessionVersionRecord>(["session", sessionId]);
      if (previous) {
        queryClient.setQueryData<SessionVersionRecord>(["session", sessionId], {
          ...previous,
          session: { ...previous.session, status: "failed" },
        });
      }
      return { previous };
    },
    onSuccess: () => {
      addToast("Session killed successfully", "success");
      onKillSuccess?.();
      onClose();
    },
    onError: (err, _variables, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["session", sessionId], context.previous);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to kill session",
        "error",
      );
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["session", sessionId] });
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
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
    <Modal open={open} onClose={handleClose} title="Kill Session">
      <div className={styles.body}>
        <p className={styles.warning}>
          Are you sure you want to kill this session? This will terminate the
          running session and cannot be undone.
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
            {mutation.isPending ? "Killing..." : "Kill Session"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
