import { useQuery } from "@tanstack/react-query";
import type { JobSummaryRecord } from "@metis/api";
import { apiClient } from "../../api/client";

/**
 * Fetches all sessions and groups them by spawned_from issue ID.
 * Used by the dashboard to show session status indicators on each issue row.
 * SSE events update the "allSessions" query key for real-time updates.
 */
export function useAllSessions() {
  return useQuery({
    queryKey: ["allSessions"],
    queryFn: () => apiClient.listSessions({ status: "created,pending,running" }),
    select: (data): Map<string, JobSummaryRecord[]> => {
      const map = new Map<string, JobSummaryRecord[]>();
      for (const session of data.jobs) {
        const issueId = session.task.spawned_from;
        if (!issueId) continue;
        const list = map.get(issueId);
        if (list) {
          list.push(session);
        } else {
          map.set(issueId, [session]);
        }
      }
      for (const list of map.values()) {
        list.sort((a, b) => a.timestamp.localeCompare(b.timestamp));
      }
      return map;
    },
    staleTime: 30_000,
  });
}
