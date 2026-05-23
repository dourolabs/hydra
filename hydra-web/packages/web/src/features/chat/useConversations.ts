import { useQuery } from "@tanstack/react-query";
import type {
  ConversationId,
  SearchConversationsQuery,
  SessionEvent,
} from "@hydra/api";
import { apiClient } from "../../api/client";

export function useConversations(
  query?: Partial<SearchConversationsQuery>,
  options?: { enabled?: boolean },
) {
  return useQuery({
    queryKey: ["conversations", query],
    queryFn: () => apiClient.listConversations(query),
    enabled: options?.enabled ?? true,
  });
}

export function useConversation(conversationId: string) {
  return useQuery({
    queryKey: ["conversation", conversationId],
    queryFn: () => apiClient.getConversation(conversationId),
    enabled: !!conversationId,
  });
}

export function useConversationEvents(conversationId: string) {
  return useQuery({
    queryKey: ["conversationEvents", conversationId],
    queryFn: () => apiClient.getConversationEvents(conversationId),
    enabled: !!conversationId,
  });
}

/**
 * GET /v1/sessions/:sessionId/events — single-session SessionEvent log.
 *
 * Counterpart to {@link useConversationEvents}; used by the chat read path
 * to fan-out fetch event logs across a conversation's resumption chain. See
 * `designs/sessions-orthogonality-redesign.md` §3.4.1.
 */
export function useSessionEvents(sessionId: string, options?: { enabled?: boolean }) {
  return useQuery<SessionEvent[]>({
    queryKey: ["sessionEvents", sessionId],
    queryFn: () => apiClient.getSessionEvents(sessionId),
    enabled: (options?.enabled ?? true) && !!sessionId,
  });
}

/**
 * List the sessions linked to a conversation, ordered by creation time
 * ascending so callers can concatenate per-session event logs in resumption
 * order.
 */
export function useSessionsByConversation(conversationId: string) {
  return useQuery({
    queryKey: ["sessionsByConversation", conversationId],
    queryFn: () =>
      apiClient.listSessions({
        conversation_id: conversationId as unknown as ConversationId,
      }),
    enabled: !!conversationId,
  });
}
