import { useCallback, useEffect, useState } from "react";

const DRAFT_KEY_PREFIX = "conversation-draft:";

function draftKey(conversationId: string): string {
  return `${DRAFT_KEY_PREFIX}${conversationId}`;
}

function readDraft(conversationId: string): string {
  if (typeof window === "undefined" || !conversationId) return "";
  try {
    return window.localStorage.getItem(draftKey(conversationId)) ?? "";
  } catch {
    return "";
  }
}

function writeDraft(conversationId: string, value: string): void {
  if (typeof window === "undefined" || !conversationId) return;
  try {
    if (value) {
      window.localStorage.setItem(draftKey(conversationId), value);
    } else {
      window.localStorage.removeItem(draftKey(conversationId));
    }
  } catch {
    /* localStorage unavailable or quota exceeded; ignore */
  }
}

export function clearConversationDraft(conversationId: string): void {
  if (typeof window === "undefined" || !conversationId) return;
  try {
    window.localStorage.removeItem(draftKey(conversationId));
  } catch {
    /* localStorage unavailable; ignore */
  }
}

export function conversationDraftKey(conversationId: string): string {
  return draftKey(conversationId);
}

export function useConversationDraft(conversationId: string): {
  value: string;
  setValue: (next: string) => void;
  clear: () => void;
} {
  const [value, setValueState] = useState<string>(() => readDraft(conversationId));

  // When the conversation id changes (route param changes without remount),
  // reload the draft for the new conversation so each chat keeps its own.
  useEffect(() => {
    setValueState(readDraft(conversationId));
  }, [conversationId]);

  const setValue = useCallback(
    (next: string) => {
      setValueState(next);
      writeDraft(conversationId, next);
    },
    [conversationId],
  );

  const clear = useCallback(() => {
    setValueState("");
    clearConversationDraft(conversationId);
  }, [conversationId]);

  return { value, setValue, clear };
}
