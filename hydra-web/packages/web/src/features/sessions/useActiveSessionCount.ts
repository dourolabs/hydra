import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

const ACTIVE_STATUSES = "created,pending,running";

export function useActiveSessionCount(creator: string | null) {
  return useQuery({
    queryKey: ["sessions", "activeCount", creator],
    queryFn: async () => {
      const resp = await apiClient.listSessions({
        status: ACTIVE_STATUSES,
        creator,
        count: true,
        limit: 1,
      });
      return Number(resp.total_count ?? 0);
    },
    enabled: !!creator,
  });
}
