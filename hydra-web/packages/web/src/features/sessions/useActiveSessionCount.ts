import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

const ACTIVE_STATUSES = "created,pending,running";

export function useActiveSessionCount() {
  return useQuery({
    queryKey: ["sessions", "activeCount"],
    queryFn: async () => {
      const resp = await apiClient.listSessions({
        status: ACTIVE_STATUSES,
        count: true,
        limit: 1,
      });
      return Number(resp.total_count ?? 0);
    },
  });
}
