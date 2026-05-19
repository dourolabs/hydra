import { useCallback, type KeyboardEvent } from "react";
import { Button, Kbd } from "@hydra/ui";
import { useConversationDraft } from "./useConversationDraft";
import styles from "./ChatInput.module.css";

interface ChatInputProps {
  conversationId: string;
  onSend: (content: string) => void;
  disabled?: boolean;
  onEndChat?: () => void;
  endChatDisabled?: boolean;
}

export function ChatInput({
  conversationId,
  onSend,
  disabled,
  onEndChat,
  endChatDisabled,
}: ChatInputProps) {
  const { value, setValue, clear } = useConversationDraft(conversationId);

  const isDisabled = disabled;

  const handleSend = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || isDisabled) return;
    onSend(trimmed);
    clear();
  }, [value, isDisabled, onSend, clear]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

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
            <Kbd>↵</Kbd> to send · <Kbd>⇧</Kbd>
            <Kbd>↵</Kbd> for newline
          </span>
          <span className={styles.actionsSpacer} />
          {onEndChat && (
            <Button variant="secondary" size="sm" onClick={onEndChat} disabled={endChatDisabled}>
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
