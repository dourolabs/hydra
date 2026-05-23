import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import type {
  ConversationEvent,
  ConversationId,
  SessionEvent,
  SessionSummaryRecord,
} from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Render-only adapter: project a {@link SessionEvent} onto the existing
 * {@link ConversationEvent} shape. `tool_use` (out of scope for Phase C per
 * design §3.7) and forward-compat `unknown` are dropped — the chat view
 * does not surface them yet.
 *
 * `SessionEvent.resumed` carries `from_session_id`; `ConversationEvent.resumed`
 * carries `session_id`. The chat renderer only reads the timestamp, so we
 * forward `from_session_id` into `session_id` to keep a single render path.
 */
export function sessionEventToConversationEvent(
  event: SessionEvent,
): ConversationEvent | null {
  switch (event.type) {
    case "user_message":
      return {
        type: "user_message",
        content: event.content,
        timestamp: event.timestamp,
      };
    case "assistant_message":
      return {
        type: "assistant_message",
        content: event.content,
        timestamp: event.timestamp,
      };
    case "suspending":
      return {
        type: "suspending",
        reason: event.reason,
        timestamp: event.timestamp,
      };
    case "resumed":
      return {
        type: "resumed",
        session_id: event.from_session_id,
        timestamp: event.timestamp,
      };
    case "closed":
      return { type: "closed", timestamp: event.timestamp };
    case "tool_use":
    case "unknown":
      return null;
  }
}

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

export type ChatTranscriptSource = "session_events" | "conversation_events";

export interface ChatTranscriptResult {
  events: ConversationEvent[];
  /**
   * Which read path produced the events:
   *  - `session_events`: at least one session in the conversation's
   *    resumption chain returned a non-empty `SessionEvent` log; the
   *    merged result is rendered.
   *  - `conversation_events`: legacy fallback (no `SessionEvent` rows in
   *    any linked session, or the conversation has no linked sessions yet).
   */
  source: ChatTranscriptSource;
  isLoading: boolean;
  error: unknown;
}

/**
 * Conversation read path per `designs/sessions-orthogonality-redesign.md`
 * §3.4.1: list sessions for the conversation, parallel fan-out fetch their
 * `SessionEvent` logs, and concatenate in creation-time order.
 *
 * Falls back to `GET /v1/conversations/:id/events` (legacy
 * `ConversationEvent` rendering) when the merged `SessionEvent` view is
 * empty — i.e. for conversations whose sessions all predate the dual-write
 * rollout. This is the cut-over described in design step 11; removing the
 * legacy path entirely is step 18.
 */
export function useChatTranscript(conversationId: string): ChatTranscriptResult {
  const sessionsQuery = useQuery({
    queryKey: ["sessionsByConversation", conversationId],
    queryFn: () =>
      apiClient.listSessions({
        conversation_id: conversationId as unknown as ConversationId,
      }),
    enabled: !!conversationId,
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
    })),
  });

  // Inlined (not `useMemo`) because `useQueries` returns a fresh outer array
  // on every render even when no underlying data changed — memoizing on it
  // would recompute every render anyway. The merge is a cheap O(N) loop, so
  // recomputing each render is acceptable and avoids a stale-reference foot
  // gun if a future contributor adds extra deps to a memo here.
  const mergedSessionEvents: ConversationEvent[] = [];
  for (const q of sessionEventQueries) {
    if (!q.data) continue;
    for (const e of q.data) {
      const projected = sessionEventToConversationEvent(e);
      if (projected) mergedSessionEvents.push(projected);
    }
  }

  // The fallback gate must wait for the sessions-list query to resolve;
  // otherwise the first render (orderedSessionIds === []) would fire the
  // legacy `GET /v1/conversations/:id/events` before we even know whether
  // this conversation has linked sessions.
  const sessionFetchesSettled =
    !sessionsQuery.isPending &&
    (orderedSessionIds.length === 0 ||
      sessionEventQueries.every((q) => !q.isPending));
  const sessionEventsEmpty = mergedSessionEvents.length === 0;

  // Legacy fallback: read ConversationEvent only after the SessionEvent
  // fan-out has settled and produced nothing. Gating prevents a spurious
  // hit on /v1/conversations/:id/events for new sessions on every render.
  const conversationEventsQuery = useQuery({
    queryKey: ["conversationEvents", conversationId],
    queryFn: () => apiClient.getConversationEvents(conversationId),
    enabled: !!conversationId && sessionFetchesSettled && sessionEventsEmpty,
  });

  if (!sessionEventsEmpty) {
    return {
      events: mergedSessionEvents,
      source: "session_events",
      isLoading:
        sessionsQuery.isLoading ||
        sessionEventQueries.some((q) => q.isLoading),
      error:
        sessionsQuery.error ??
        sessionEventQueries.find((q) => q.error)?.error ??
        null,
    };
  }

  return {
    events: conversationEventsQuery.data ?? [],
    source: "conversation_events",
    isLoading:
      sessionsQuery.isLoading ||
      sessionEventQueries.some((q) => q.isLoading) ||
      conversationEventsQuery.isLoading,
    error:
      sessionsQuery.error ??
      sessionEventQueries.find((q) => q.error)?.error ??
      conversationEventsQuery.error ??
      null,
  };
}
