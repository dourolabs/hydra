import type { ConversationSummary } from "@hydra/api";

// Post `p-wrmctcvh` the backend's `last_event_preview` is sourced from the
// lifecycle-only `ConversationEvent` log, so an untitled chat that ended
// before a title was generated surfaces as e.g. "Suspending: sigterm" in the
// list. Reject these lifecycle previews (and any empty/whitespace candidate)
// so we fall through to a neutral placeholder.
const LIFECYCLE_PREVIEW = /^(?:Suspending:|Suspended:|Resumed$|Closed$)/;

export function conversationTitle(c: ConversationSummary): string {
  if (c.title) return c.title;
  const preview = c.last_event_preview?.trim();
  if (preview && !LIFECYCLE_PREVIEW.test(preview)) {
    return preview;
  }
  return "Untitled conversation";
}
