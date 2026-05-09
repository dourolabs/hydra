import { useState, useCallback } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Spinner } from "@hydra/ui";
import type { Conversation, ConversationEvent } from "@hydra/api";
import { useConversation, useConversationEvents } from "../features/chat/useConversations";
import { ChatHeader } from "../features/chat/ChatHeader";
import { ChatMessageList } from "../features/chat/ChatMessageList";
import { ChatInput } from "../features/chat/ChatInput";
import { ApiError, apiClient } from "../api/client";
import styles from "./ChatPage.module.css";

function NewChatPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [creating, setCreating] = useState(false);

  const createMutation = useMutation({
    mutationFn: (message: string) =>
      apiClient.createConversation({ message }),
    onSuccess: (conversation: Conversation) => {
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate(`/chat/${conversation.conversation_id}`, { replace: true });
    },
    onSettled: () => setCreating(false),
  });

  const handleSend = useCallback(
    (content: string) => {
      if (creating) return;
      setCreating(true);
      createMutation.mutate(content);
    },
    [creating, createMutation],
  );

  return (
    <div className={styles.chatLayout}>
      <div className={styles.newHeader}>
        <button className={styles.back} onClick={() => navigate("/chat")}>
          &larr; Chat
        </button>
        <h2 className={styles.title}>New conversation</h2>
      </div>
      <ChatMessageList events={[]} />
      <ChatInput onSend={handleSend} disabled={creating} />
    </div>
  );
}

function ExistingChatPage({ conversationId }: { conversationId: string }) {
  const queryClient = useQueryClient();
  const { data: conversation, isLoading, error } = useConversation(conversationId);
  const { data: events } = useConversationEvents(conversationId);

  const sendMutation = useMutation({
    mutationFn: (content: string) =>
      apiClient.sendMessage(conversationId, { content }),
    onMutate: async (content) => {
      await queryClient.cancelQueries({ queryKey: ["conversationEvents", conversationId] });
      const previous = queryClient.getQueryData<ConversationEvent[]>(["conversationEvents", conversationId]);
      const optimisticEvent: ConversationEvent = {
        type: "user_message",
        content,
        timestamp: new Date().toISOString(),
      };
      queryClient.setQueryData<ConversationEvent[]>(
        ["conversationEvents", conversationId],
        (old) => [...(old ?? []), optimisticEvent],
      );
      return { previous };
    },
    onError: (_err, _content, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["conversationEvents", conversationId], context.previous);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["conversationEvents", conversationId] });
    },
  });

  const handleSend = useCallback(
    (content: string) => {
      sendMutation.mutate(content);
    },
    [sendMutation],
  );

  if (isLoading) {
    return (
      <div className={styles.chatLayout}>
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      </div>
    );
  }

  if (error) {
    const is404 = error instanceof ApiError && error.status === 404;
    return (
      <div className={styles.chatLayout}>
        <div className={styles.errorContainer}>
          <p className={styles.error}>
            {is404
              ? `Conversation ${conversationId} not found.`
              : `Failed to load conversation: ${(error as Error).message}`}
          </p>
        </div>
      </div>
    );
  }

  if (!conversation) return null;

  return (
    <div className={styles.chatLayout}>
      <ChatHeader conversation={conversation} />
      <ChatMessageList events={events ?? []} />
      <ChatInput
        onSend={handleSend}
        disabled={sendMutation.isPending}
        status={conversation.status}
      />
    </div>
  );
}

export function ChatPage() {
  const { conversationId } = useParams<{ conversationId: string }>();

  if (conversationId === "new") {
    return <NewChatPage />;
  }

  return <ExistingChatPage conversationId={conversationId ?? ""} />;
}
