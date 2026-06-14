import { useCallback, useLayoutEffect, useRef, type KeyboardEvent } from "react";
import { Button, Icons } from "@hydra/ui";
import { useIsMobile } from "../../hooks/useIsMobile";
import { useConversationDraft } from "./useConversationDraft";
import styles from "./ChatInput.module.css";

// Sized to comfortably hold the corner Send button at empty state. The Send
// button picks up Button's iconOnly sizing (28×28 desktop, 44×44 mobile via
// the touch-target floor in Button.module.css), so the floor differs by
// breakpoint and the JS MIN tracks that to avoid the button overhanging the
// textarea edge.
const MIN_HEIGHT_DESKTOP_PX = 36;
const MIN_HEIGHT_MOBILE_PX = 52;
const MAX_HEIGHT_PX = 480;
// The textarea inherits the global `box-sizing: border-box`, so its `height`
// includes the 1px top + 1px bottom border. scrollHeight reports content +
// padding only, so we add the border thickness back when sizing — otherwise
// each set-height would shrink the visible content area by 2px and clip the
// last line of text.
const BORDER_PX = 2;

interface ChatInputProps {
  conversationId: string;
  onSend: (content: string) => void;
  disabled?: boolean;
}

export function ChatInput({ conversationId, onSend, disabled }: ChatInputProps) {
  const { value, setValue, clear } = useConversationDraft(conversationId);

  const isDisabled = disabled;
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const isMobile = useIsMobile();
  const minHeightPx = isMobile ? MIN_HEIGHT_MOBILE_PX : MIN_HEIGHT_DESKTOP_PX;

  // Auto-grow: re-measure scrollHeight whenever the value changes and clamp
  // the textarea height to [MIN, MAX]. Resetting to MIN_HEIGHT first lets
  // scrollHeight shrink back down when the user deletes lines.
  useLayoutEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = `${minHeightPx}px`;
    const next = Math.max(
      minHeightPx,
      Math.min(MAX_HEIGHT_PX, el.scrollHeight + BORDER_PX),
    );
    el.style.height = `${next}px`;
  }, [value, minHeightPx]);

  const handleSend = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || isDisabled) return;
    onSend(trimmed);
    clear();
  }, [value, isDisabled, onSend, clear]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      // On mobile, let Enter insert a newline (use the Send button instead).
      // The soft keyboard's Return key would otherwise submit unexpectedly.
      if (isMobile) return;
      if (e.key === "Enter" && !e.shiftKey && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend, isMobile],
  );

  const sendDisabled = isDisabled || !value.trim();

  return (
    <div className={styles.composer}>
      <div className={styles.inner}>
        <div className={styles.textareaWrapper}>
          <textarea
            ref={textareaRef}
            className={styles.textarea}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type a message…"
            disabled={isDisabled}
            rows={1}
          />
          <Button
            className={styles.sendButton}
            variant="primary"
            onClick={handleSend}
            disabled={sendDisabled}
            aria-label="Send"
            title={isMobile ? undefined : "↵ to send · ⇧↵ for newline"}
          >
            <Icons.IconSend size={16} />
          </Button>
        </div>
      </div>
    </div>
  );
}
