import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

const ACTIVE_STATUSES = "created,pending,running";

export function useActiveSessions(creator: string | null, limit = 6) {
  return useQuery({
    queryKey: ["sessions", "active", creator, limit],
    queryFn: async () => {
      const resp = await apiClient.listSessions({
        status: ACTIVE_STATUSES,
        creator,
        limit,
      });
      return resp.sessions;
    },
    enabled: !!creator,
  });
}
