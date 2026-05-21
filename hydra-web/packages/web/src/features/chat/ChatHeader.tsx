import type { Conversation } from "@hydra/api";
import { AgoTime } from "../../components/Runtime/Runtime";
import styles from "./ChatHeader.module.css";

interface ChatHeaderProps {
  conversation: Conversation;
}

export function ChatHeader({ conversation }: ChatHeaderProps) {
  const title = conversation.title || "Untitled conversation";

  return (
    <div className={styles.header}>
      <div className={styles.inner}>
        <h1 className={styles.title}>{title}</h1>
        <div className={styles.meta} data-testid="chat-header-meta">
          {conversation.agent_name && (
            <>
              <span>with {conversation.agent_name}</span>
              <span className={styles.sep}>·</span>
            </>
          )}
          <span>opened by {conversation.creator}</span>
          <span className={styles.sep}>·</span>
          <span data-testid="chat-header-started">
            started <AgoTime iso={conversation.created_at} />
          </span>
          <span className={styles.sep}>·</span>
          <span>{conversation.status}</span>
        </div>
      </div>
    </div>
  );
}
