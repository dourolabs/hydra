import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import type { Issue, IssueVersionRecord, UpsertIssueRequest } from "@metis/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";

export function useIssue(issueId: string) {
  return useQuery({
    queryKey: ["issue", issueId],
    queryFn: () => apiClient.getIssue(issueId),
    enabled: !!issueId,
  });
}

export function useUpdateIssue(issueId: string) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();

  return useMutation({
    mutationFn: (updatedIssue: Issue) => {
      const request: UpsertIssueRequest = {
        issue: updatedIssue,
        job_id: null,
      };
      return apiClient.updateIssue(issueId, request);
    },
    onMutate: async (updatedIssue: Issue) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: updatedIssue,
        });
      }
      return { previous };
    },
    onError: (err, _updatedIssue, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to update issue",
        "error",
      );
    },
    onSuccess: () => {
      addToast("Issue updated", "success");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });
}
