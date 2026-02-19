import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useJobsByIssue(issueId: string) {
  return useQuery({
    queryKey: ["jobs", issueId],
    queryFn: () => apiClient.listJobs({ spawned_from: issueId }),
    select: (data) => data.jobs,
    enabled: !!issueId,
  });
}
