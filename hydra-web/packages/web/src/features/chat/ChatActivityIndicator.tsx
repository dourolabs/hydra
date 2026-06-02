import { Spinner } from "@hydra/ui";
import type { ActivityStatus } from "./deriveActivityStatus";
import styles from "./ChatActivityIndicator.module.css";

interface ChatActivityIndicatorProps {
  status: ActivityStatus;
}

/**
 * Transient activity indicator rendered directly below the chat thread (NOT
 * as a transcript row). Hidden by its parent when `deriveActivityStatus`
 * returns `null`.
 *
 * Styled as italic / subdued, with a small `Spinner size="sm"` to the left
 * and the status text to the right. The fallback `Using <tool_name>` shape
 * splits the tool name into a separate inline-code element.
 */
export function ChatActivityIndicator({ status }: ChatActivityIndicatorProps) {
  return (
    <div
      className={styles.indicator}
      data-testid="chat-activity-indicator"
      role="status"
      aria-live="polite"
    >
      <div className={styles.inner}>
        <Spinner size="sm" />
        <span className={styles.text} data-testid="chat-activity-indicator-text">
          {status.text}
          {status.toolName ? (
            <code className={styles.code}>{status.toolName}</code>
          ) : null}
        </span>
      </div>
    </div>
  );
}
