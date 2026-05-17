import { useState } from "react";
import type { Conversation } from "@hydra/api";
import { ChatMetadataTab } from "./ChatMetadataTab";
import { ChatRelatedTab } from "./ChatRelatedTab";
import styles from "./ChatRightPanel.module.css";

export type ChatRightPanelTabKey = "related" | "settings";

const TABS: { key: ChatRightPanelTabKey; label: string }[] = [
  { key: "related", label: "Related" },
  { key: "settings", label: "Settings" },
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
  const activeTab = isControlled ? activeTabKey : internalTab;

  const handleTabClick = (key: ChatRightPanelTabKey) => {
    if (!isControlled) setInternalTab(key);
    onTabChange?.(key);
  };

  return (
    <aside className={styles.wrapper} data-mobile-active={dataMobileActive}>
      <div className={styles.tabs} role="tablist">
        {TABS.map((t) => (
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
        {activeTab === "settings" && <ChatMetadataTab conversation={conversation} />}
      </div>
    </aside>
  );
}
