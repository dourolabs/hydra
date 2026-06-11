import { Link } from "react-router-dom";
import type { Conversation } from "@hydra/api";
import { AgoTime } from "../../components/Runtime/Runtime";
import styles from "./ChatHeader.module.css";

interface ChatHeaderProps {
  conversation: Conversation;
}

export function ChatHeader({ conversation }: ChatHeaderProps) {
  return (
    <div className={styles.header} data-testid="chat-header">
      <div className={styles.inner}>
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
          {conversation.spawned_from && (
            <>
              <span className={styles.sep}>·</span>
              <span data-testid="chat-header-originated-from">
                originated from{" "}
                <Link
                  to={`/issues/${conversation.spawned_from}`}
                  className={styles.originatedLink}
                >
                  {conversation.spawned_from}
                </Link>
              </span>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
