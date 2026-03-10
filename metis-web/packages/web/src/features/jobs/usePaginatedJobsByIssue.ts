import { useInfiniteQuery } from "@tanstack/react-query";
import type { ListJobsResponse } from "@metis/api";
import { apiClient } from "../../api/client";

const DEFAULT_PAGE_SIZE = 25;

/**
 * Fetches jobs for a specific issue page-by-page using cursor-based pagination.
 */
export function usePaginatedJobsByIssue(issueId: string, limit?: number) {
  const pageSize = limit ?? DEFAULT_PAGE_SIZE;

  return useInfiniteQuery<ListJobsResponse, Error>({
    queryKey: ["paginatedJobs", issueId],
    queryFn: ({ pageParam }) =>
      apiClient.listJobs({
        spawned_from: issueId,
        limit: pageSize,
        cursor: (pageParam as string) ?? null,
      }),
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    initialPageParam: null as string | null,
    enabled: !!issueId,
  });
}
