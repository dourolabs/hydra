import type { ConversationEvent } from "@hydra/api";

/**
 * Layer locally-optimistic `user_message` events on top of the server-side
 * transcript, dropping each optimistic entry at most once when a real event
 * with the same content has arrived.
 *
 * Counting (rather than set membership) lets duplicate sends of the same
 * text reconcile one-for-one rather than collapsing into a single entry.
 *
 * Used by ChatPage so the optimistic user_message stays rendered through the
 * mutation's `onSettled` invalidation refetch window — clearing local state
 * synchronously caused a visible flicker where the just-sent message
 * disappeared until the refetch resolved.
 */
export function mergeOptimisticEvents(
  transcript: readonly ConversationEvent[],
  optimistic: readonly ConversationEvent[],
): ConversationEvent[] {
  const remaining = new Map<string, number>();
  for (const e of transcript) {
    if (e.type === "user_message") {
      remaining.set(e.content, (remaining.get(e.content) ?? 0) + 1);
    }
  }
  const pending: ConversationEvent[] = [];
  for (const e of optimistic) {
    if (e.type === "user_message") {
      const left = remaining.get(e.content) ?? 0;
      if (left > 0) {
        remaining.set(e.content, left - 1);
        continue;
      }
    }
    pending.push(e);
  }
  return [...transcript, ...pending];
}
