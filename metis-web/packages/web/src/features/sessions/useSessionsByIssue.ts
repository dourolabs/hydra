import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useSessionsByIssue(issueId: string) {
  return useQuery({
    queryKey: ["sessions", issueId],
    queryFn: () => apiClient.listSessions({ spawned_from: issueId }),
    select: (data) => data.jobs,
    enabled: !!issueId,
  });
}
