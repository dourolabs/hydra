import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

const ACTIVE_STATUSES = "created,pending,running";

export function useActiveSessions(limit = 6) {
  return useQuery({
    queryKey: ["sessions", "active", limit],
    queryFn: async () => {
      const resp = await apiClient.listSessions({
        status: ACTIVE_STATUSES,
        limit,
      });
      return resp.sessions;
    },
  });
}
