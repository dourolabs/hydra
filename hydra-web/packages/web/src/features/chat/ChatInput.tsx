import { useState, useCallback, type KeyboardEvent } from "react";
import { Button } from "@hydra/ui";
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
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  return (
    <div className={styles.inputBar}>
      <textarea
        className={styles.textarea}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Type a message..."
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
      {onEndChat && (
        <Button
          variant="danger"
          size="sm"
          onClick={onEndChat}
          disabled={endChatDisabled}
        >
          End Chat
        </Button>
      )}
    </div>
  );
}
