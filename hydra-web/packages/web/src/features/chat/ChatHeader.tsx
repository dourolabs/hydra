import { Button } from "@hydra/ui";
import type { Conversation } from "@hydra/api";
import { AgoTime } from "../../components/Runtime/Runtime";
import styles from "./ChatHeader.module.css";

interface ChatHeaderProps {
  conversation: Conversation;
  onEndChat?: () => void;
  endChatDisabled?: boolean;
}

export function ChatHeader({ conversation, onEndChat, endChatDisabled }: ChatHeaderProps) {
  const title = conversation.title || "Untitled conversation";

  return (
    <div
      className={styles.header}
      data-testid="chat-header"
      data-has-action={onEndChat ? "true" : "false"}
    >
      <div className={styles.inner}>
        <div className={styles.headLeft}>
          <h1 className={styles.title}>{title}</h1>
          <div className={styles.meta} data-testid="chat-header-meta">
            {conversation.agent_name && (
              <>
                <span>with {conversation.agent_name}</span>
                <span className={styles.sep}>·</span>
              </>
            )}
            <span data-testid="chat-header-started">
              started <AgoTime iso={conversation.created_at} />
            </span>
            <span className={styles.sep}>·</span>
            <span>{conversation.status}</span>
          </div>
        </div>
        {onEndChat && (
          <div className={styles.headRight}>
            <Button variant="secondary" size="sm" onClick={onEndChat} disabled={endChatDisabled}>
              End chat
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
