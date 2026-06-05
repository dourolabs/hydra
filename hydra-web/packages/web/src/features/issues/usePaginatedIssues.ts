import { keepPreviousData, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type {
  IssueStatus,
  IssueSummaryRecord,
  ListIssuesResponse,
  SearchIssuesQuery,
} from "@hydra/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

export interface IssueFilters {
  status?: string | null;
  type?: string | null;
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
  if (filters.type) query.issue_type = filters.type as SearchIssuesQuery["issue_type"];
  if (filters.creator) query.creator = filters.creator;
  if (filters.assignee) query.assignee = filters.assignee;
  if (filters.labels) query.labels = filters.labels;
  if (filters.q) query.q = filters.q;
  if (filters.ids) query.ids = filters.ids;
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
    placeholderData: keepPreviousData,
    enabled,
  });
}

// Board columns are fixed; the rules of hooks forbid useInfiniteQuery in a
// loop, so the per-status hook below makes 5 named calls.
export const BOARD_STATUSES = [
  "open",
  "in-progress",
  "failed",
  "closed",
  "dropped",
] as const satisfies readonly IssueStatus[];

export type BoardStatus = (typeof BOARD_STATUSES)[number];

export interface BoardColumnQuery {
  issues: IssueSummaryRecord[];
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  fetchNextPage: () => void;
}

function dedupeIssues(issues: IssueSummaryRecord[]): IssueSummaryRecord[] {
  const seen = new Set<string>();
  const out: IssueSummaryRecord[] = [];
  for (const issue of issues) {
    if (seen.has(issue.issue_id)) continue;
    seen.add(issue.issue_id);
    out.push(issue);
  }
  return out;
}

/**
 * Per-status paginated board hook. Fires one `useInfiniteQuery` per column so
 * each column can be deep-paginated independently. Each query key shares the
 * `["paginatedIssues", …]` prefix used by the table-view query, so SSE
 * invalidations propagate to both shapes.
 *
 * When the page's chip filter is active (`baseFilters.status` set), every
 * column queries with that status (sharing the matching column's cache via a
 * common queryKey). Non-matching columns then render empty after a render-side
 * filter, matching the table-mode semantics where a chip narrows to a single
 * status.
 */
export function usePaginatedIssuesByStatus(
  baseFilters: IssueFilters,
  enabled = true,
): Record<BoardStatus, BoardColumnQuery> {
  const chipStatus = (baseFilters.status ?? null) as BoardStatus | null;

  function makeQueryKey(column: BoardStatus) {
    const effective = chipStatus ?? column;
    return ["paginatedIssues", { ...baseFilters, status: effective }] as const;
  }

  function makeQueryFn(column: BoardStatus) {
    const effective = chipStatus ?? column;
    return ({ pageParam }: { pageParam: unknown }) =>
      apiClient.listIssues(
        buildQuery(
          { ...baseFilters, status: effective },
          pageParam as string | undefined,
        ),
      );
  }

  const openQuery = useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: makeQueryKey("open"),
    queryFn: makeQueryFn("open"),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
  const inProgressQuery = useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: makeQueryKey("in-progress"),
    queryFn: makeQueryFn("in-progress"),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
  const failedQuery = useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: makeQueryKey("failed"),
    queryFn: makeQueryFn("failed"),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
  const closedQuery = useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: makeQueryKey("closed"),
    queryFn: makeQueryFn("closed"),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
  const droppedQuery = useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: makeQueryKey("dropped"),
    queryFn: makeQueryFn("dropped"),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });

  return useMemo(() => {
    function toColumn(
      column: BoardStatus,
      query:
        | typeof openQuery
        | typeof inProgressQuery
        | typeof failedQuery
        | typeof closedQuery
        | typeof droppedQuery,
    ): BoardColumnQuery {
      const filteredOut = chipStatus !== null && chipStatus !== column;
      const raw = query.data?.pages.flatMap((p) => p.issues) ?? [];
      // When the chip filter contradicts this column, render no rows even
      // though the shared query may have data — the column header still shows.
      const visible = filteredOut
        ? []
        : dedupeIssues(raw).filter((rec) => rec.issue.status === column);
      return {
        issues: visible,
        isLoading: !filteredOut && query.isLoading,
        hasNextPage: !filteredOut && (query.hasNextPage ?? false),
        isFetchingNextPage: query.isFetchingNextPage ?? false,
        fetchNextPage: () => {
          if (query.hasNextPage && !query.isFetchingNextPage) {
            query.fetchNextPage();
          }
        },
      };
    }
    return {
      open: toColumn("open", openQuery),
      "in-progress": toColumn("in-progress", inProgressQuery),
      failed: toColumn("failed", failedQuery),
      closed: toColumn("closed", closedQuery),
      dropped: toColumn("dropped", droppedQuery),
    };
  }, [chipStatus, openQuery, inProgressQuery, failedQuery, closedQuery, droppedQuery]);
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
      if (filters.type) query.issue_type = filters.type as SearchIssuesQuery["issue_type"];
      if (filters.creator) query.creator = filters.creator;
      if (filters.assignee) query.assignee = filters.assignee;
      if (filters.labels) query.labels = filters.labels;
      if (filters.q) query.q = filters.q;
      if (filters.ids) query.ids = filters.ids;
      const resp = await apiClient.listIssues(query);
      return Number(resp.total_count ?? 0);
    },
    placeholderData: keepPreviousData,
    enabled,
  });
}
