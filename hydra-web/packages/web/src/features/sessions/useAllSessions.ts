import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

/**
 * Fetch every session in the system (no spawned_from filter), used by the
 * `/sessions` list page. Server-side status filter is omitted so terminal
 * sessions are included; ordering is applied client-side via
 * `sortSessionsActiveFirst`.
 */
export function useAllSessions() {
  return useQuery({
    queryKey: ["sessions", "all"],
    queryFn: () => apiClient.listSessions(),
    select: (data) => data.sessions,
  });
}
