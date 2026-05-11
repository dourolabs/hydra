import { useState, useCallback } from "react";
import { useParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Tabs } from "@hydra/ui";
import type { Conversation, ConversationEvent } from "@hydra/api";
import { useConversation, useConversationEvents } from "../features/chat/useConversations";
import { ChatHeader } from "../features/chat/ChatHeader";
import { ChatMessageList } from "../features/chat/ChatMessageList";
import { ChatInput } from "../features/chat/ChatInput";
import { IssueSettings } from "../features/issues/IssueSettings";
import { formatTimestamp } from "../utils/time";
import { ApiError, apiClient } from "../api/client";
import styles from "./ChatPage.module.css";

const TABS = [
  { id: "chat", label: "Chat" },
  { id: "details", label: "Details" },
];

function ConversationDetails({ conversation }: { conversation: Conversation }) {
  return (
    <div className={styles.detailsTab}>
      <div className={styles.meta}>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Conversation ID</span>
          <span className={styles.metaValue}>{conversation.conversation_id}</span>
        </div>
        {conversation.title && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Title</span>
            <span className={styles.metaValue}>{conversation.title}</span>
          </div>
        )}
        {conversation.agent_name && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Agent</span>
            <span className={styles.metaValue}>{conversation.agent_name}</span>
          </div>
        )}
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Status</span>
          <span className={styles.metaValue}>{conversation.status}</span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Creator</span>
          <span className={styles.metaValue}>{conversation.creator}</span>
        </div>
        {conversation.active_session_id && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Active Session</span>
            <span className={styles.metaValue}>{conversation.active_session_id}</span>
          </div>
        )}
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Created</span>
          <span className={styles.metaValue}>{formatTimestamp(conversation.created_at)}</span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>{formatTimestamp(conversation.updated_at)}</span>
        </div>
      </div>
      <IssueSettings jobSettings={conversation.session_settings} />
    </div>
  );
}

function ExistingChatPage({ conversationId }: { conversationId: string }) {
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState("chat");
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
      <Panel
        header={
          <Tabs
            tabs={TABS}
            activeTab={activeTab}
            onTabChange={setActiveTab}
          />
        }
      >
        {activeTab === "chat" && (
          <div className={styles.chatTabContent}>
            <ChatHeader conversation={conversation} />
            <ChatMessageList events={events ?? []} />
            <ChatInput
              onSend={handleSend}
              disabled={sendMutation.isPending}
              status={conversation.status}
            />
          </div>
        )}
        {activeTab === "details" && (
          <div className={styles.sectionBody}>
            <ConversationDetails conversation={conversation} />
          </div>
        )}
      </Panel>
    </div>
  );
}

export function ChatPage() {
  const { conversationId } = useParams<{ conversationId: string }>();
  return <ExistingChatPage conversationId={conversationId ?? ""} />;
}
