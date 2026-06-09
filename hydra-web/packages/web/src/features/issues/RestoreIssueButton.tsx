import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type { IssueVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";

interface RestoreIssueButtonProps {
  issueId: string;
  className?: string;
  "data-testid"?: string;
}

interface RestoreContext {
  previousIssue?: IssueVersionRecord;
}

/**
 * Row-level "Restore" action shown on archived issue rows. Re-fetches the
 * full issue (`include_deleted=true`) before submitting the update so the
 * PUT body preserves fields the summary record drops (session_settings,
 * form, form_response, feedback). Without that round-trip the update would
 * blank those fields out on the server.
 */
export function RestoreIssueButton({
  issueId,
  className,
  "data-testid": testId,
}: RestoreIssueButtonProps) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();

  const mutation = useMutation<unknown, Error, void, RestoreContext>({
    mutationFn: async () => {
      const full = await apiClient.getIssue(issueId, true);
      return apiClient.updateIssue(issueId, {
        issue: {
          ...full.issue,
          status: full.issue.status.key,
          deleted: false,
        },
        session_id: null,
      });
    },
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previousIssue = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previousIssue) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previousIssue,
          issue: { ...previousIssue.issue, deleted: false },
        });
      }
      return { previousIssue };
    },
    onError: (err, _variables, context) => {
      if (context?.previousIssue) {
        queryClient.setQueryData(["issue", issueId], context.previousIssue);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to restore issue",
        "error",
      );
    },
    onSuccess: () => {
      addToast("Issue restored", "success");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
      queryClient.invalidateQueries({ queryKey: ["issueCount"] });
    },
  });

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      className={className}
      disabled={mutation.isPending}
      aria-label="Restore archived issue"
      data-testid={testId}
      onClick={(e) => {
        e.stopPropagation();
        e.preventDefault();
        if (!mutation.isPending) mutation.mutate();
      }}
    >
      {mutation.isPending ? "Restoring…" : "Restore"}
    </Button>
  );
}
