import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Spinner } from "@hydra/ui";
import type { Conversation, SessionEvent } from "@hydra/api";
import { useConversation } from "./useConversations";
import { useUsername } from "../auth/useUsername";
import { useChatTranscript } from "./useChatTranscript";
import { mergeOptimisticEvents } from "./mergeOptimisticEvents";
import { ChatHeader } from "./ChatHeader";
import { ChatMessageList } from "./ChatMessageList";
import { deriveActivitySteps } from "./deriveActivitySteps";
import { ChatInput } from "./ChatInput";
import { clearConversationDraft } from "./useConversationDraft";
import { ChatRightPanel, type ChatRightPanelTabKey } from "./ChatRightPanel";
import { MobileTabBar, type MobileTabBarItem } from "../../components/MobileTabBar";
import { useConversationProxyStatus } from "../../hooks/useConversationProxyStatus";
import { ApiError, apiClient } from "../../api/client";
import { useBreadcrumbs } from "../../layout/useBreadcrumbs";
import styles from "./ExistingChatPage.module.css";

type MobileTabKey = "chat" | ChatRightPanelTabKey;

const BASE_MOBILE_TABS: MobileTabBarItem[] = [
  { key: "chat", label: "Chat" },
  { key: "related", label: "Related" },
  { key: "details", label: "Details" },
];

const PROXY_MOBILE_TAB: MobileTabBarItem = { key: "proxy", label: "Proxy" };

export function ExistingChatPage({ conversationId }: { conversationId: string }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data: conversation, isLoading, error } = useConversation(conversationId);
  const transcript = useChatTranscript(conversationId);
  const currentUsername = useUsername();

  const [mobileTab, setMobileTab] = useState<MobileTabKey>("chat");
  const [rightPanelTab, setRightPanelTab] = useState<ChatRightPanelTabKey>("related");
  // Hoisted here so the mobile tab bar and the right panel agree on whether
  // the Proxy tab is visible. The hook returns cached data — calling it in
  // ChatRightPanel too is cheap.
  const proxyStatus = useConversationProxyStatus(conversationId);
  const proxyAvailable = proxyStatus.targets.length > 0;
  // Local optimistic events live outside the query cache so they layer on
  // top of the SessionEvent transcript. Entries are reconciled away in the
  // `events` merge below as soon as their server-side counterpart lands in
  // `transcript.events`, so the message never flickers between the mutation
  // settling and the refetch completing.
  const [optimisticEvents, setOptimisticEvents] = useState<SessionEvent[]>([]);

  useBreadcrumbs(
    [
      { label: "Workspace", to: "/" },
      { label: "Chats", to: "/chat" },
    ],
    conversation?.title || conversationId,
  );

  const events = useMemo<SessionEvent[]>(
    () => mergeOptimisticEvents(transcript.events, optimisticEvents),
    [transcript.events, optimisticEvents],
  );

  const sendMutation = useMutation({
    mutationFn: (content: string) => apiClient.sendMessage(conversationId, { content }),
    onMutate: (content) => {
      const optimistic = {
        type: "user_message",
        content,
        timestamp: new Date().toISOString(),
      } satisfies SessionEvent;
      setOptimisticEvents((prev) => [...prev, optimistic]);
      return { optimistic };
    },
    onError: (_err, _content, context) => {
      const failed = context?.optimistic;
      if (failed) {
        setOptimisticEvents((prev) => prev.filter((e) => e !== failed));
      }
    },
    onSettled: () => {
      // Refetch the per-session SessionEvent logs (and the sessions list, in
      // case send-on-closed conversation spawned a fresh session). Clearing
      // the optimistic entry is left to the `events` merge above, which
      // drops it once the refetched transcript contains the real event —
      // this avoids the flicker that occurred when synchronously clearing
      // local state here raced the refetch.
      queryClient.invalidateQueries({ queryKey: ["sessionsByConversation", conversationId] });
      queryClient.invalidateQueries({ queryKey: ["sessionEvents"] });
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
    onSuccess: () => {
      clearConversationDraft(conversationId);
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
      case "proxy":
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
  const activity = deriveActivitySteps(events, conversation.status);

  // Inserted as the 3rd mobile tab (between Related and Details) when the
  // active session has advertised any proxy targets — same hide-when-empty
  // rule as the desktop right-panel tab.
  const mobileTabs: MobileTabBarItem[] = proxyAvailable
    ? [BASE_MOBILE_TABS[0], BASE_MOBILE_TABS[1], PROXY_MOBILE_TAB, BASE_MOBILE_TABS[2]]
    : BASE_MOBILE_TABS;
  const activeMobileTab: MobileTabKey =
    mobileTab === "proxy" && !proxyAvailable ? "chat" : mobileTab;

  return (
    <div className={styles.chatLayout}>
      <MobileTabBar
        className={styles.mobileTabBar}
        tabs={mobileTabs}
        activeKey={activeMobileTab}
        onChange={handleMobileTabChange}
        testIdPrefix="chat-mobile-tab-"
      />
      <div
        className={styles.chatPane}
        data-mobile-active={chatPaneActive ? "true" : "false"}
        data-testid="chat-pane"
      >
        <ChatHeader conversation={conversation} />
        <ChatMessageList
          events={events}
          agentName={conversation.agent_name}
          creator={conversation.creator}
          currentUsername={currentUsername}
          activity={activity}
        />
        <ChatInput
          conversationId={conversationId}
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
