import type { Conversation } from "@hydra/api";
import { IssueSettings } from "../issues/IssueSettings";
import { formatTimestamp } from "../../utils/time";
import styles from "./ChatMetadataTab.module.css";

interface ChatMetadataTabProps {
  conversation: Conversation;
}

export function ChatMetadataTab({ conversation }: ChatMetadataTabProps) {
  return (
    <div className={styles.metadataTab}>
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
          <span className={styles.metaLabel}>Creator</span>
          <span className={styles.metaValue}>{conversation.creator}</span>
        </div>
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
