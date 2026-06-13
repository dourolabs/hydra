import {
  useMutation,
  useQueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import type { IssueVersionRecord, ListIssuesResponse } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";

// The `["paginatedIssues"]` prefix is shared by two query shapes:
//   - usePaginatedIssues (table view) — `InfiniteData<ListIssuesResponse>`
//   - useBoardIssuesByProject (board view) — `ListIssuesResponse[]`
// A single setQueriesData call has to handle both.
type PaginatedIssuesCache =
  | InfiniteData<ListIssuesResponse>
  | ListIssuesResponse[];

interface ArchiveContext {
  previousIssue?: IssueVersionRecord;
  paginatedSnapshots: Array<[readonly unknown[], PaginatedIssuesCache | undefined]>;
}

function dropIssueFromPage(
  page: ListIssuesResponse,
  issueId: string,
): ListIssuesResponse {
  const idx = page.issues.findIndex((rec) => rec.issue_id === issueId);
  if (idx === -1) return page;
  const issues = page.issues.slice(0, idx).concat(page.issues.slice(idx + 1));
  return { ...page, issues };
}

/**
 * Soft-delete an issue via `DELETE /v1/issues/:id`. Drops the row from every
 * paginated cache so it leaves the default (non-archived) view immediately;
 * the include-archived view re-renders the row with its ARCHIVED tag after
 * the refetch. The detail cache flips `archived: true` so a viewer on the
 * detail page sees the Archived badge appear instantly.
 */
export function useArchiveIssue(issueId: string) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();

  const mutation = useMutation<unknown, Error, void, ArchiveContext>({
    mutationFn: () => apiClient.deleteIssue(issueId),
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
          issue: { ...previousIssue.issue, archived: true },
        });
      }

      // Drop the row from every paginated cache so it disappears from the
      // default view immediately. The include-archived view's refetch will
      // re-add the row as ARCHIVED — accepting a brief blink there in
      // exchange for instant drop in the dominant (default) view.
      const paginatedSnapshots = queryClient.getQueriesData<PaginatedIssuesCache>({
        queryKey: ["paginatedIssues"],
      });
      queryClient.setQueriesData<PaginatedIssuesCache>(
        { queryKey: ["paginatedIssues"] },
        (old) => {
          if (!old) return old;
          if (Array.isArray(old)) {
            return old.map((page) => dropIssueFromPage(page, issueId));
          }
          return {
            ...old,
            pages: old.pages.map((page) => dropIssueFromPage(page, issueId)),
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
        err instanceof Error ? err.message : "Failed to archive issue",
        "error",
      );
    },
    onSuccess: () => {
      addToast("Issue archived", "success");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
      queryClient.invalidateQueries({ queryKey: ["issueCount"] });
    },
  });

  return {
    archive: () => {
      if (!mutation.isPending) mutation.mutate();
    },
    isPending: mutation.isPending,
  };
}
