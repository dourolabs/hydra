import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type { Conversation } from "@hydra/api";
import { apiClient } from "../../api/client";
import styles from "./ChatHeader.module.css";

interface ChatHeaderProps {
  conversation: Conversation;
}

export function ChatHeader({ conversation }: ChatHeaderProps) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const closeMutation = useMutation({
    mutationFn: () => apiClient.closeConversation(conversation.conversation_id),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["conversation", conversation.conversation_id] });
      const previous = queryClient.getQueryData<Conversation>(["conversation", conversation.conversation_id]);
      queryClient.setQueryData<Conversation>(
        ["conversation", conversation.conversation_id],
        (old) => old ? { ...old, status: "closed" as const } : old,
      );
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["conversation", conversation.conversation_id], context.previous);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["conversation", conversation.conversation_id] });
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate("/chat");
    },
  });

  const title = conversation.title || "Untitled conversation";
  const canClose = conversation.status !== "closed";

  return (
    <div className={styles.header}>
      <div className={styles.titleRow}>
        <button className={styles.back} onClick={() => navigate("/chat")}>
          &larr; Chat
        </button>
        <h2 className={styles.title}>{title}</h2>
      </div>
      <div className={styles.actions}>
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
