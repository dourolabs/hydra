import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import type { Issue } from "@metis/api";
import { apiClient } from "../../api/client";

export function useIssue(issueId: string) {
  return useQuery({
    queryKey: ["issue", issueId],
    queryFn: () => apiClient.getIssue(issueId),
    enabled: !!issueId,
  });
}

export function useUpdateIssue(issueId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (issue: Issue) =>
      apiClient.updateIssue(issueId, { issue, job_id: null }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });
}
