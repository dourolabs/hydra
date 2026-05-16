import type { Conversation } from "@hydra/api";
import { formatRelativeTime } from "../../utils/time";
import styles from "./ChatHeader.module.css";

interface ChatHeaderProps {
  conversation: Conversation;
}

export function ChatHeader({ conversation }: ChatHeaderProps) {
  const title = conversation.title || "Untitled conversation";
  const created = formatRelativeTime(conversation.created_at);

  return (
    <div className={styles.header}>
      <div className={styles.inner}>
        <h1 className={styles.title}>{title}</h1>
        <div className={styles.meta}>
          {conversation.agent_name && (
            <>
              <span>with {conversation.agent_name}</span>
              <span className={styles.sep}>·</span>
            </>
          )}
          <span>opened by {conversation.creator}</span>
          <span className={styles.sep}>·</span>
          <span>started {created}</span>
          <span className={styles.sep}>·</span>
          <span>{conversation.status}</span>
        </div>
      </div>
    </div>
  );
}
