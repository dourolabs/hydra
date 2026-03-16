import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { SearchIssuesQuery, ListIssuesResponse } from "@metis/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

export interface IssueFilters {
  status?: string | null;
  creator?: string | null;
  assignee?: string | null;
  labels?: string | null;
  q?: string | null;
  ids?: string | null;
}

function buildQuery(
  filters: IssueFilters,
  cursor?: string | null,
): Partial<SearchIssuesQuery> {
  const query: Partial<SearchIssuesQuery> = {
    limit: PAGE_SIZE,
  };
  if (filters.status) query.status = filters.status as SearchIssuesQuery["status"];
  if (filters.creator) query.creator = filters.creator;
  if (filters.assignee) query.assignee = filters.assignee;
  if (filters.labels) query.labels = filters.labels;
  if (filters.q) query.q = filters.q;
  if (cursor) query.cursor = cursor;
  return query;
}

/**
 * Paginated issues hook using cursor-based pagination with React Query's
 * useInfiniteQuery. Query key includes all active filters so changing
 * filters automatically refetches.
 */
export function usePaginatedIssues(filters: IssueFilters, enabled = true) {
  return useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: ["paginatedIssues", filters],
    queryFn: ({ pageParam }) =>
      apiClient.listIssues(buildQuery(filters, pageParam as string | undefined)),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    enabled,
  });
}

/**
 * Count-only query for badge counts. Uses limit=0 and count=true
 * to get total_count without fetching issue data.
 */
export function useIssueCount(filters: IssueFilters, enabled = true) {
  return useQuery({
    queryKey: ["issueCount", filters],
    queryFn: async () => {
      const query: Partial<SearchIssuesQuery> = {
        limit: 0,
        count: true,
      };
      if (filters.status) query.status = filters.status as SearchIssuesQuery["status"];
      if (filters.creator) query.creator = filters.creator;
      if (filters.assignee) query.assignee = filters.assignee;
      if (filters.labels) query.labels = filters.labels;
      if (filters.q) query.q = filters.q;
      if (filters.ids) query.ids = filters.ids;
      const resp = await apiClient.listIssues(query);
      return Number(resp.total_count ?? 0);
    },
    enabled,
  });
}
