import { useState, useCallback, type KeyboardEvent } from "react";
import { Button, Kbd } from "@hydra/ui";
import styles from "./ChatInput.module.css";

interface ChatInputProps {
  onSend: (content: string) => void;
  disabled?: boolean;
  onEndChat?: () => void;
  endChatDisabled?: boolean;
}

export function ChatInput({
  onSend,
  disabled,
  onEndChat,
  endChatDisabled,
}: ChatInputProps) {
  const [value, setValue] = useState("");

  const isDisabled = disabled;

  const handleSend = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || isDisabled) return;
    onSend(trimmed);
    setValue("");
  }, [value, isDisabled, onSend]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      // Match the issue-create modal: ⌘/Ctrl+Enter submits, plain Enter is a
      // newline (textarea default).
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const isMac = typeof navigator !== "undefined" && navigator.platform.includes("Mac");

  return (
    <div className={styles.composer}>
      <div className={styles.inner}>
        <textarea
          className={styles.textarea}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Type a message…"
          disabled={isDisabled}
          rows={3}
        />
        <div className={styles.actions}>
          <span className={styles.hint}>
            <Kbd>{isMac ? "⌘" : "Ctrl"}</Kbd>
            <Kbd>↵</Kbd> to send · <Kbd>↵</Kbd> for newline
          </span>
          <span className={styles.actionsSpacer} />
          {onEndChat && (
            <Button
              variant="secondary"
              size="sm"
              onClick={onEndChat}
              disabled={endChatDisabled}
            >
              End chat
            </Button>
          )}
          <Button
            variant="primary"
            size="sm"
            onClick={handleSend}
            disabled={isDisabled || !value.trim()}
          >
            Send
          </Button>
        </div>
      </div>
    </div>
  );
}
