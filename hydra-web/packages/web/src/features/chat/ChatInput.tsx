import { useState, useCallback, type KeyboardEvent } from "react";
import { Button } from "@hydra/ui";
import type { ConversationStatus } from "@hydra/api";
import styles from "./ChatInput.module.css";

interface ChatInputProps {
  onSend: (content: string) => void;
  disabled?: boolean;
  status?: ConversationStatus;
}

export function ChatInput({ onSend, disabled, status }: ChatInputProps) {
  const [value, setValue] = useState("");

  const isDisabled = disabled || status === "idle" || status === "closed";

  const handleSend = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || isDisabled) return;
    onSend(trimmed);
    setValue("");
  }, [value, isDisabled, onSend]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const placeholder = isDisabled ? "Resume to continue" : "Type a message...";

  return (
    <div className={styles.inputBar}>
      <textarea
        className={styles.textarea}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        disabled={isDisabled}
        rows={1}
      />
      <Button
        variant="primary"
        size="sm"
        onClick={handleSend}
        disabled={isDisabled || !value.trim()}
      >
        Send
      </Button>
    </div>
  );
}
