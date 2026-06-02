import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

/**
 * GET /v1/sessions/:sessionId/events.
 *
 * Live-tail is handled by `useSSE`: `session_event_created` events
 * invalidate this query key so new events appear without a refetch loop here.
 */
export function useSessionEvents(sessionId: string) {
  return useQuery({
    queryKey: ["sessionEvents", sessionId],
    queryFn: () => apiClient.getSessionEvents(sessionId),
    enabled: !!sessionId,
  });
}
