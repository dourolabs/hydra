import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Badge, Button } from "@hydra/ui";
import type { Conversation, ConversationStatus } from "@hydra/api";
import { apiClient } from "../../api/client";
import { normalizeConversationStatus } from "../../utils/statusMapping";
import styles from "./ChatHeader.module.css";

function statusLabel(status: ConversationStatus): string {
  switch (status) {
    case "active":
      return "Active";
    case "idle":
      return "Idle";
    case "closed":
      return "Closed";
  }
}

interface ChatHeaderProps {
  conversation: Conversation;
}

export function ChatHeader({ conversation }: ChatHeaderProps) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const closeMutation = useMutation({
    mutationFn: () => apiClient.closeConversation(conversation.conversation_id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["conversation", conversation.conversation_id] });
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate("/chat");
    },
  });

  const resumeMutation = useMutation({
    mutationFn: () => apiClient.resumeConversation(conversation.conversation_id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["conversation", conversation.conversation_id] });
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
    },
  });

  const title = conversation.title || "Untitled conversation";
  const canResume = conversation.status === "idle" || conversation.status === "closed";
  const canClose = conversation.status !== "closed";

  return (
    <div className={styles.header}>
      <div className={styles.titleRow}>
        <button className={styles.back} onClick={() => navigate("/chat")}>
          &larr; Chat
        </button>
        <h2 className={styles.title}>{title}</h2>
        <div className={styles.status}>
          <Badge status={normalizeConversationStatus(conversation.status)} />
          <span className={styles.statusLabel}>{statusLabel(conversation.status)}</span>
        </div>
      </div>
      <div className={styles.actions}>
        {canResume && (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => resumeMutation.mutate()}
            disabled={resumeMutation.isPending}
          >
            Resume
          </Button>
        )}
        {canClose && (
          <Button
            variant="danger"
            size="sm"
            onClick={() => closeMutation.mutate()}
            disabled={closeMutation.isPending}
          >
            End Chat
          </Button>
        )}
      </div>
    </div>
  );
}
