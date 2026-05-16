import { keepPreviousData, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { ListSessionsResponse, SearchSessionsQuery } from "@hydra/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

export interface SessionFilters {
  status?: string | null;
}

function buildQuery(
  filters: SessionFilters,
  cursor?: string | null,
): Partial<SearchSessionsQuery> {
  const query: Partial<SearchSessionsQuery> = {
    limit: PAGE_SIZE,
  };
  if (filters.status) query.status = filters.status;
  if (cursor) query.cursor = cursor;
  return query;
}

/**
 * Paginated sessions hook using cursor-based pagination with React Query's
 * useInfiniteQuery. Query key includes filters so changing filters
 * automatically refetches from page 1.
 */
export function usePaginatedSessions(filters: SessionFilters, enabled = true) {
  return useInfiniteQuery<ListSessionsResponse, Error>({
    queryKey: ["paginatedSessions", filters],
    queryFn: ({ pageParam }) =>
      apiClient.listSessions(buildQuery(filters, pageParam as string | undefined)),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
}

/**
 * Count-only query for the page eyebrow total. Uses limit=0 and count=true
 * to get total_count without fetching session data.
 */
export function useSessionCount(filters: SessionFilters, enabled = true) {
  return useQuery({
    queryKey: ["sessionCount", filters],
    queryFn: async () => {
      const query: Partial<SearchSessionsQuery> = {
        limit: 0,
        count: true,
      };
      if (filters.status) query.status = filters.status;
      const resp = await apiClient.listSessions(query);
      return Number(resp.total_count ?? 0);
    },
    enabled,
  });
}
