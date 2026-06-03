import { Spinner } from "@hydra/ui";
import type { ActivityStep } from "./deriveActivitySteps";
import styles from "./ChatActivityIndicator.module.css";

interface ChatActivityIndicatorProps {
  current: ActivityStep;
}

/**
 * Transient activity indicator rendered directly below the chat thread (NOT
 * as a transcript row). Hidden by its parent when the derived activity has
 * no active step.
 *
 * Styled as italic / subdued, with a small `Spinner size="sm"` to the left
 * and the status text to the right. When a tool call carries a description,
 * that description wins over the generic verb; the fallback
 * `Using <tool_name>` shape splits the tool name into a separate inline-code
 * element.
 */
export function ChatActivityIndicator({ current }: ChatActivityIndicatorProps) {
  const text = current.detail ?? current.verb;
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
          {text}
          {current.toolName ? (
            <code className={styles.code}>{current.toolName}</code>
          ) : null}
        </span>
      </div>
    </div>
  );
}
