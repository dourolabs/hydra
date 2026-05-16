import { useState } from "react";
import type { Conversation } from "@hydra/api";
import { ChatMetadataTab } from "./ChatMetadataTab";
import { ChatRelatedTab } from "./ChatRelatedTab";
import styles from "./ChatRightPanel.module.css";

type TabKey = "related" | "settings";

const TABS: { key: TabKey; label: string }[] = [
  { key: "related", label: "Related" },
  { key: "settings", label: "Settings" },
];

interface ChatRightPanelProps {
  conversation: Conversation;
}

export function ChatRightPanel({ conversation }: ChatRightPanelProps) {
  const [activeTab, setActiveTab] = useState<TabKey>("related");

  return (
    <aside className={styles.wrapper}>
      <div className={styles.tabs} role="tablist">
        {TABS.map((t) => (
          <button
            key={t.key}
            type="button"
            role="tab"
            className={`${styles.tab}${activeTab === t.key ? ` ${styles.tabActive}` : ""}`}
            aria-selected={activeTab === t.key}
            onClick={() => setActiveTab(t.key)}
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
