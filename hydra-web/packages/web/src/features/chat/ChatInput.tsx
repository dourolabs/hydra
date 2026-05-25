import {
  useCallback,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { Button, Kbd } from "@hydra/ui";
import { useConversationDraft } from "./useConversationDraft";
import styles from "./ChatInput.module.css";

const MIN_HEIGHT_PX = 56;
const MAX_HEIGHT_PX = 480;

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
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [height, setHeight] = useState<number | null>(null);

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

  const handleResizePointerDown = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
    const textareaEl = textareaRef.current;
    if (!textareaEl) return;
    if (e.button !== 0) return;
    e.preventDefault();

    const startY = e.clientY;
    const startHeight = textareaEl.offsetHeight;

    const handleMove = (ev: PointerEvent) => {
      // Dragging up (negative delta) grows the textarea; the composer is
      // anchored to the bottom of the chat pane so the top edge visually
      // moves up while the action bar stays put.
      const delta = ev.clientY - startY;
      const next = Math.max(MIN_HEIGHT_PX, Math.min(MAX_HEIGHT_PX, startHeight - delta));
      setHeight(next);
    };

    const handleUp = () => {
      window.removeEventListener("pointermove", handleMove);
      window.removeEventListener("pointerup", handleUp);
      window.removeEventListener("pointercancel", handleUp);
    };

    window.addEventListener("pointermove", handleMove);
    window.addEventListener("pointerup", handleUp);
    window.addEventListener("pointercancel", handleUp);
  }, []);

  return (
    <div className={styles.composer}>
      <div className={styles.inner}>
        <div className={styles.textareaWrapper}>
          <div
            className={styles.resizeHandle}
            role="separator"
            aria-orientation="horizontal"
            aria-label="Resize chat input"
            onPointerDown={handleResizePointerDown}
            data-testid="chat-input-resize-handle"
          />
          <textarea
            ref={textareaRef}
            className={styles.textarea}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type a message…"
            disabled={isDisabled}
            rows={3}
            style={height != null ? { height: `${height}px` } : undefined}
          />
        </div>
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
