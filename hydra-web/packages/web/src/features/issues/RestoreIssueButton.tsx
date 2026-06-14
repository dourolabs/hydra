import {
  useMutation,
  useQueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type {
  IssueSummaryRecord,
  IssueVersionRecord,
  ListIssuesResponse,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";

interface RestoreIssueButtonProps {
  issueId: string;
  className?: string;
  "data-testid"?: string;
}

// The `["paginatedIssues"]` prefix is shared by two query shapes:
//   - usePaginatedIssues (table view) — `InfiniteData<ListIssuesResponse>`
//   - useBoardIssuesByProject (board view) — `ListIssuesResponse[]`
// A single setQueriesData call has to handle both.
type PaginatedIssuesCache =
  | InfiniteData<ListIssuesResponse>
  | ListIssuesResponse[];

interface RestoreContext {
  previousIssue?: IssueVersionRecord;
  paginatedSnapshots: Array<[readonly unknown[], PaginatedIssuesCache | undefined]>;
}

function clearDeletedOnRecord(rec: IssueSummaryRecord): IssueSummaryRecord {
  if (rec.issue.archived !== true) return rec;
  return { ...rec, issue: { ...rec.issue, archived: false } };
}

function clearDeletedOnPage(
  page: ListIssuesResponse,
  issueId: string,
): ListIssuesResponse {
  let mutated = false;
  const issues = page.issues.map((rec) => {
    if (rec.issue_id !== issueId) return rec;
    const next = clearDeletedOnRecord(rec);
    if (next !== rec) mutated = true;
    return next;
  });
  return mutated ? { ...page, issues } : page;
}

/**
 * Row-level "Restore" action shown on archived issue rows. Re-fetches the
 * full issue (`include_archived=true`) before submitting the update so the
 * PUT body preserves fields the summary record drops (session_settings,
 * form, form_response). Without that round-trip the update would
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
          archived: false,
        },
        session_id: null,
      });
    },
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      await queryClient.cancelQueries({ queryKey: ["paginatedIssues"] });

      const previousIssue = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previousIssue) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previousIssue,
          issue: { ...previousIssue.issue, archived: false },
        });
      }

      // The list cache drives the row-level ARCHIVED tag (via
      // IssueSummaryRecord.issue.archived), not the detail cache. Invalidating
      // alone leaves the tag on the row until the refetch lands, so flip the
      // record in every matching cache page now.
      const paginatedSnapshots = queryClient.getQueriesData<PaginatedIssuesCache>({
        queryKey: ["paginatedIssues"],
      });
      queryClient.setQueriesData<PaginatedIssuesCache>(
        { queryKey: ["paginatedIssues"] },
        (old) => {
          if (!old) return old;
          if (Array.isArray(old)) {
            return old.map((page) => clearDeletedOnPage(page, issueId));
          }
          return {
            ...old,
            pages: old.pages.map((page) => clearDeletedOnPage(page, issueId)),
          };
        },
      );

      return { previousIssue, paginatedSnapshots };
    },
    onError: (err, _variables, context) => {
      if (context?.previousIssue) {
        queryClient.setQueryData(["issue", issueId], context.previousIssue);
      }
      if (context) {
        for (const [key, data] of context.paginatedSnapshots) {
          queryClient.setQueryData(key, data);
        }
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
