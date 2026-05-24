import type { ConversationSummary } from "@hydra/api";

export function conversationTitle(c: ConversationSummary): string {
  if (c.title) return c.title;
  const preview = c.last_event_preview?.trim();
  if (preview) return preview;
  return "Untitled conversation";
}
