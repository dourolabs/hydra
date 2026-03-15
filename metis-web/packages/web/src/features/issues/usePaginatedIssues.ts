import { useInfiniteQuery, useQuery, keepPreviousData } from "@tanstack/react-query";
import type { SearchIssuesQuery, ListIssuesResponse } from "@metis/api";
import { apiClient } from "../../api/client";

export type IssueFilters = Partial<Omit<SearchIssuesQuery, "limit" | "cursor" | "count">>;

export function usePaginatedIssues(
  filters: IssueFilters = {},
  limit: number = 50,
) {
  const query = useInfiniteQuery<ListIssuesResponse, Error, { pages: ListIssuesResponse[]; pageParams: (string | undefined)[] }, unknown[], string | undefined>({
    queryKey: ["issues", "paginated", filters, limit],
    queryFn: ({ pageParam }) =>
      apiClient.listIssues({
        ...filters,
        limit,
        cursor: pageParam ?? null,
      } as Partial<SearchIssuesQuery>),
    initialPageParam: undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
  });

  const issues = query.data?.pages.flatMap((page) => page.issues) ?? [];

  return {
    ...query,
    issues,
  };
}

export function useIssueCount(filters: IssueFilters = {}) {
  return useQuery({
    queryKey: ["issues", "count", filters],
    queryFn: () =>
      apiClient.listIssues({
        ...filters,
        limit: 0,
        count: true,
      } as Partial<SearchIssuesQuery>),
    select: (data) => (data.total_count != null ? Number(data.total_count) : 0),
    placeholderData: keepPreviousData,
  });
}
