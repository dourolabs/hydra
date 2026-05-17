import { useCallback, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Spinner } from "@hydra/ui";
import type { Conversation, ConversationEvent } from "@hydra/api";
import { useConversation, useConversationEvents } from "../features/chat/useConversations";
import { ChatHeader } from "../features/chat/ChatHeader";
import { ChatMessageList } from "../features/chat/ChatMessageList";
import { ChatInput } from "../features/chat/ChatInput";
import { ChatRightPanel, type ChatRightPanelTabKey } from "../features/chat/ChatRightPanel";
import { MobileTabBar, type MobileTabBarItem } from "../components/MobileTabBar";
import { ApiError, apiClient } from "../api/client";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./ChatPage.module.css";

type MobileTabKey = "chat" | ChatRightPanelTabKey;

const MOBILE_TABS: MobileTabBarItem[] = [
  { key: "chat", label: "Chat" },
  { key: "related", label: "Related" },
  { key: "details", label: "Details" },
];

function ExistingChatPage({ conversationId }: { conversationId: string }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data: conversation, isLoading, error } = useConversation(conversationId);
  const { data: events } = useConversationEvents(conversationId);

  const [mobileTab, setMobileTab] = useState<MobileTabKey>("chat");
  const [rightPanelTab, setRightPanelTab] = useState<ChatRightPanelTabKey>("related");

  useBreadcrumbs(
    [
      { label: "Workspace", to: "/" },
      { label: "Chats", to: "/chat" },
    ],
    conversation?.title || conversationId,
  );

  const sendMutation = useMutation({
    mutationFn: (content: string) => apiClient.sendMessage(conversationId, { content }),
    onMutate: async (content) => {
      await queryClient.cancelQueries({ queryKey: ["conversationEvents", conversationId] });
      const previous = queryClient.getQueryData<ConversationEvent[]>([
        "conversationEvents",
        conversationId,
      ]);
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
      queryClient.invalidateQueries({ queryKey: ["conversation", conversationId] });
    },
  });

  const closeMutation = useMutation({
    mutationFn: () => apiClient.closeConversation(conversationId),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["conversation", conversationId] });
      const previous = queryClient.getQueryData<Conversation>(["conversation", conversationId]);
      queryClient.setQueryData<Conversation>(["conversation", conversationId], (old) =>
        old ? { ...old, status: "closed" as const } : old,
      );
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["conversation", conversationId], context.previous);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["conversation", conversationId] });
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate("/chat");
    },
  });

  const handleSend = useCallback(
    (content: string) => {
      sendMutation.mutate(content);
    },
    [sendMutation],
  );

  const handleEndChat = useCallback(() => {
    closeMutation.mutate();
  }, [closeMutation]);

  const handleMobileTabChange = useCallback((key: string) => {
    switch (key) {
      case "chat":
        setMobileTab("chat");
        return;
      case "related":
      case "details":
        setMobileTab(key);
        setRightPanelTab(key);
        return;
    }
  }, []);

  const handleRightPanelChange = useCallback((key: ChatRightPanelTabKey) => {
    setRightPanelTab(key);
  }, []);

  if (isLoading) {
    return (
      <div className={styles.statusLayout}>
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      </div>
    );
  }

  if (error) {
    const is404 = error instanceof ApiError && error.status === 404;
    return (
      <div className={styles.statusLayout}>
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

  const canClose = conversation.status !== "closed";
  const chatPaneActive = mobileTab === "chat";

  return (
    <div className={styles.chatLayout}>
      <MobileTabBar
        className={styles.mobileTabBar}
        tabs={MOBILE_TABS}
        activeKey={mobileTab}
        onChange={handleMobileTabChange}
        testIdPrefix="chat-mobile-tab-"
      />
      <div className={styles.chatPane} data-mobile-active={chatPaneActive ? "true" : "false"}>
        <ChatHeader conversation={conversation} />
        <ChatMessageList events={events ?? []} agentName={conversation.agent_name} />
        <ChatInput
          onSend={handleSend}
          disabled={sendMutation.isPending}
          onEndChat={canClose ? handleEndChat : undefined}
          endChatDisabled={closeMutation.isPending}
        />
      </div>
      <ChatRightPanel
        conversation={conversation}
        activeTabKey={rightPanelTab}
        onTabChange={handleRightPanelChange}
        data-mobile-active={chatPaneActive ? "false" : "true"}
      />
    </div>
  );
}

export function ChatPage() {
  const { conversationId } = useParams<{ conversationId: string }>();
  return <ExistingChatPage conversationId={conversationId ?? ""} />;
}
