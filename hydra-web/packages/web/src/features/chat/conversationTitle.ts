import type { ConversationSummary } from "@hydra/api";

export function conversationTitle(c: ConversationSummary): string {
  return c.title || c.last_event_preview || "Untitled conversation";
}
