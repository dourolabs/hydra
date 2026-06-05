import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import type {
  ConversationId,
  SessionEvent,
  SessionSummaryRecord,
} from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Order session-summary records by creation time ascending so concatenating
 * their event logs reproduces the chronological transcript. Sessions
 * without a `creation_time` are pushed to the end stably.
 *
 * The design assumes session N+1 is created strictly after session N has
 * emitted `Suspending` (or been `Closed`), so the per-session event vectors
 * never interleave across sessions — concatenation is sufficient.
 */
function sessionsInResumptionOrder(
  records: readonly SessionSummaryRecord[],
): SessionSummaryRecord[] {
  return [...records].sort((a, b) => {
    const at = a.session.creation_time ?? "";
    const bt = b.session.creation_time ?? "";
    return at.localeCompare(bt);
  });
}

export interface ChatTranscriptResult {
  events: SessionEvent[];
  isLoading: boolean;
  error: unknown;
}

/**
 * Conversation read path per `designs/sessions-orthogonality-redesign.md`
 * §3.4.1: list sessions for the conversation, parallel fan-out fetch their
 * `SessionEvent` logs, and concatenate in creation-time order.
 *
 * Phase E step 18 retired the legacy `ConversationEvent` fallback: chat
 * content lives exclusively on `SessionEvent` now, so the only read path
 * is the per-session fan-out.
 */
export function useChatTranscript(conversationId: string): ChatTranscriptResult {
  const sessionsQuery = useQuery({
    queryKey: ["sessionsByConversation", conversationId],
    queryFn: () =>
      apiClient.listSessions({
        conversation_id: conversationId as unknown as ConversationId,
      }),
    enabled: !!conversationId,
    // Force a fresh fetch on every chat-page mount so the activity indicator
    // (derived from the events tail) is reconciled with the server on
    // navigate-back, not whatever the cache held when the user last navigated
    // away. The global `staleTime: 30_000` (App.tsx) plus missable SSE
    // invalidations (disconnected EventSource, payload racing the route
    // change) can otherwise pin a stale "thinking…" tail until a hard refresh.
    refetchOnMount: "always",
  });

  const orderedSessionIds = useMemo(() => {
    const sessions = sessionsQuery.data?.sessions ?? [];
    return sessionsInResumptionOrder(sessions).map((r) => r.session_id);
  }, [sessionsQuery.data]);

  // Parallel fan-out per design §3.4.1 step 2. Each session keeps its own
  // ["sessionEvents", sid] cache key so SSE invalidations land on a single
  // session log without re-fetching the whole conversation.
  const sessionEventQueries = useQueries({
    queries: orderedSessionIds.map((sid) => ({
      queryKey: ["sessionEvents", sid],
      queryFn: () => apiClient.getSessionEvents(sid),
      enabled: !!sid,
      // Same rationale as `sessionsQuery`: the activity indicator reads off
      // the events tail, so the per-session log must refresh on remount.
      refetchOnMount: "always" as const,
    })),
  });

  // Inlined (not `useMemo`) because `useQueries` returns a fresh outer array
  // on every render even when no underlying data changed — memoizing on it
  // would recompute every render anyway. The merge is a cheap O(N) loop, so
  // recomputing each render is acceptable and avoids a stale-reference foot
  // gun if a future contributor adds extra deps to a memo here.
  const events: SessionEvent[] = [];
  for (const q of sessionEventQueries) {
    if (!q.data) continue;
    for (const e of q.data) {
      events.push(e);
    }
  }

  return {
    events,
    isLoading:
      sessionsQuery.isLoading ||
      sessionEventQueries.some((q) => q.isLoading),
    error:
      sessionsQuery.error ??
      sessionEventQueries.find((q) => q.error)?.error ??
      null,
  };
}
