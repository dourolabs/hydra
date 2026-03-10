import { useInfiniteQuery } from "@tanstack/react-query";
import type { ListIssuesResponse } from "@metis/api";
import { apiClient } from "../../api/client";

const DEFAULT_PAGE_SIZE = 50;

interface PaginatedIssuesOptions {
  q?: string;
  limit?: number;
}

/**
 * Fetches issues page-by-page using cursor-based pagination,
 * with embedded subtree and job status summary data.
 */
export function usePaginatedIssues(options?: PaginatedIssuesOptions) {
  const q = options?.q;
  const limit = options?.limit ?? DEFAULT_PAGE_SIZE;

  return useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: ["paginatedIssues", q ?? null],
    queryFn: ({ pageParam }) =>
      apiClient.listIssues({
        q: q ?? null,
        limit,
        cursor: (pageParam as string) ?? null,
        include_subtree: true,
        include_job_status: true,
      }),
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    initialPageParam: null as string | null,
    staleTime: 30_000,
  });
}
