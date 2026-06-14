import { Button } from "@hydra/ui";
import type { Conversation } from "@hydra/api";
import { AgoTime } from "../../components/Runtime/Runtime";
import styles from "./ChatHeader.module.css";

interface ChatHeaderProps {
  conversation: Conversation;
  onEndChat?: () => void;
  endChatDisabled?: boolean;
}

// Single-row desktop header: meta line on the left, End chat on the right.
// The conversation title is already rendered by SiteHeader.Breadcrumbs, so
// the dedicated H1 row would duplicate it — we keep it in the DOM as an
// `sr-only`-style clipped element for screen readers and existing tests but
// drop it from the visual layout. On mobile the entire header is hidden via
// CSS — End chat lives in the trailing slot of MobileTabBar instead.
export function ChatHeader({ conversation, onEndChat, endChatDisabled }: ChatHeaderProps) {
  const title = conversation.title || "Untitled conversation";

  return (
    <div className={styles.header} data-testid="chat-header">
      <h1 className={styles.srTitle}>{title}</h1>
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
      {onEndChat && (
        <Button
          className={styles.endChat}
          variant="secondary"
          size="sm"
          onClick={onEndChat}
          disabled={endChatDisabled}
        >
          End chat
        </Button>
      )}
    </div>
  );
}
