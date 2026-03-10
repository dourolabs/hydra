import { useInfiniteQuery } from "@tanstack/react-query";
import type { ListIssuesResponse, IssueStatus } from "@metis/api";
import { apiClient } from "../../api/client";

const DEFAULT_PAGE_SIZE = 50;

interface PaginatedIssuesOptions {
  q?: string;
  limit?: number;
  /** Filter by a single issue status. */
  status?: IssueStatus;
  /** Filter by comma-separated label IDs. */
  labels?: string;
  /** Filter by assignee username. */
  assignee?: string;
}

/**
 * Fetches issues page-by-page using cursor-based pagination,
 * with embedded subtree and job status summary data.
 *
 * Accepts optional server-side filters so each dashboard tab can
 * independently paginate over its own result set.
 */
export function usePaginatedIssues(options?: PaginatedIssuesOptions) {
  const q = options?.q;
  const limit = options?.limit ?? DEFAULT_PAGE_SIZE;
  const status = options?.status ?? null;
  const labels = options?.labels ?? null;
  const assignee = options?.assignee ?? null;

  return useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: ["paginatedIssues", { q: q ?? null, status, labels, assignee }],
    queryFn: ({ pageParam }) => {
      const query: Record<string, unknown> = {
        q: q ?? null,
        status,
        assignee,
        limit,
        cursor: (pageParam as string) ?? null,
        include_subtree: true,
        include_job_status: true,
      };
      if (labels) query.labels = labels;
      return apiClient.listIssues(query as Parameters<typeof apiClient.listIssues>[0]);
    },
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    initialPageParam: null as string | null,
    staleTime: 30_000,
  });
}
