import { useState } from "react";
import type { Conversation } from "@hydra/api";
import { ChatMetadataTab } from "./ChatMetadataTab";
import { ChatRelatedTab } from "./ChatRelatedTab";
import { ProxyTab } from "./ProxyTab";
import { useConversationProxyTargets } from "../../hooks/useConversationProxyStatus";
import styles from "./ChatRightPanel.module.css";

export type ChatRightPanelTabKey = "related" | "proxy" | "details";

interface TabDef {
  key: ChatRightPanelTabKey;
  label: string;
}

const BASE_TABS: TabDef[] = [
  { key: "related", label: "Related" },
  { key: "details", label: "Details" },
];

interface ChatRightPanelProps {
  conversation: Conversation;
  activeTabKey?: ChatRightPanelTabKey;
  onTabChange?: (key: ChatRightPanelTabKey) => void;
  "data-mobile-active"?: "true" | "false";
}

export function ChatRightPanel({
  conversation,
  activeTabKey,
  onTabChange,
  "data-mobile-active": dataMobileActive,
}: ChatRightPanelProps) {
  const [internalTab, setInternalTab] = useState<ChatRightPanelTabKey>("related");
  const isControlled = activeTabKey !== undefined;
  const requestedTab = isControlled ? activeTabKey : internalTab;

  // The Proxy tab only exists when the conversation's active session has
  // advertised one or more proxy targets. Hiding it (rather than disabling)
  // keeps the right-rail clean for the common case where no dev preview is
  // running. Use the targets-only hook here — the full status hook (with HEAD
  // probes) is mounted exactly once inside `ProxyTab`.
  const { targets: proxyTargets } = useConversationProxyTargets(
    conversation.conversation_id,
  );
  const proxyAvailable = proxyTargets.length > 0;

  const tabs: TabDef[] = proxyAvailable
    ? [BASE_TABS[0], { key: "proxy", label: "Proxy" }, BASE_TABS[1]]
    : BASE_TABS;

  // If the proxy tab disappears while it was active, fall back to `related`
  // without losing the user's place.
  const activeTab: ChatRightPanelTabKey =
    requestedTab === "proxy" && !proxyAvailable ? "related" : requestedTab;

  const handleTabClick = (key: ChatRightPanelTabKey) => {
    if (!isControlled) setInternalTab(key);
    onTabChange?.(key);
  };

  return (
    <aside className={styles.wrapper} data-mobile-active={dataMobileActive}>
      <div className={styles.tabs} role="tablist">
        {tabs.map((t) => (
          <button
            key={t.key}
            type="button"
            role="tab"
            className={`${styles.tab}${activeTab === t.key ? ` ${styles.tabActive}` : ""}`}
            aria-selected={activeTab === t.key}
            onClick={() => handleTabClick(t.key)}
            data-testid={`chat-rail-tab-${t.key}`}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className={styles.body}>
        {activeTab === "related" && (
          <ChatRelatedTab conversationId={conversation.conversation_id} />
        )}
        {activeTab === "proxy" && proxyAvailable && (
          <ProxyTab conversationId={conversation.conversation_id} />
        )}
        {activeTab === "details" && <ChatMetadataTab conversation={conversation} />}
      </div>
    </aside>
  );
}
