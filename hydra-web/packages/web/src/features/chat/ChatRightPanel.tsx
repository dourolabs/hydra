import { useState } from "react";
import { Panel, Tabs } from "@hydra/ui";
import type { Conversation } from "@hydra/api";
import { ChatMetadataTab } from "./ChatMetadataTab";
import { ChatRelatedTab } from "./ChatRelatedTab";
import styles from "./ChatRightPanel.module.css";

const TABS = [
  { id: "related", label: "Related" },
  { id: "metadata", label: "Metadata" },
];

interface ChatRightPanelProps {
  conversation: Conversation;
}

export function ChatRightPanel({ conversation }: ChatRightPanelProps) {
  const [activeTab, setActiveTab] = useState("related");

  return (
    <div className={styles.wrapper}>
      <Panel
        className={styles.panel}
        fillHeight
        header={
          <Tabs tabs={TABS} activeTab={activeTab} onTabChange={setActiveTab} />
        }
      >
        {activeTab === "related" && (
          <ChatRelatedTab conversationId={conversation.conversation_id} />
        )}
        {activeTab === "metadata" && <ChatMetadataTab conversation={conversation} />}
      </Panel>
    </div>
  );
}
