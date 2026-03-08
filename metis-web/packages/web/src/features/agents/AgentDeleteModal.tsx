import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Modal } from "@metis/ui";
import type { AgentRecord } from "@metis/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./AgentsSection.module.css";

interface AgentDeleteModalProps {
  open: boolean;
  agent: AgentRecord;
  onClose: () => void;
}

export function AgentDeleteModal({ open, agent, onClose }: AgentDeleteModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => apiClient.deleteAgent(agent.name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      addToast("Agent deleted", "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete agent",
        "error",
      );
    },
  });

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      onClose();
    }
  }, [mutation.isPending, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title="Delete Agent">
      <div className={styles.deleteModalContent}>
        <p className={styles.deleteMessage}>
          Are you sure you want to delete this agent?
        </p>
        <p className={styles.deleteAgentName}>{agent.name}</p>
        <div className={styles.deleteActions}>
          <Button
            variant="secondary"
            size="md"
            onClick={handleClose}
            disabled={mutation.isPending}
          >
            Cancel
          </Button>
          <Button
            variant="danger"
            size="md"
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? "Deleting..." : "Delete"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
