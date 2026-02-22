import { useQuery } from "@tanstack/react-query";
import type { JobVersionRecord } from "@metis/api";
import { apiClient } from "../../api/client";

/**
 * Fetches all jobs and groups them by spawned_from issue ID.
 * Used by the dashboard to show job status indicators on each issue row.
 * SSE events update the "allJobs" query key for real-time updates.
 */
export function useAllJobs() {
  return useQuery({
    queryKey: ["allJobs"],
    queryFn: () => apiClient.listJobs(),
    select: (data): Map<string, JobVersionRecord[]> => {
      const map = new Map<string, JobVersionRecord[]>();
      for (const job of data.jobs) {
        const issueId = job.task.spawned_from;
        if (!issueId) continue;
        const list = map.get(issueId);
        if (list) {
          list.push(job);
        } else {
          map.set(issueId, [job]);
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
