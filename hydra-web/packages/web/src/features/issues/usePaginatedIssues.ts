import {
  keepPreviousData,
  useInfiniteQuery,
  useQueries,
  useQuery,
} from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";
import type {
  IssueSummaryRecord,
  ListIssuesResponse,
  SearchIssuesQuery,
  StatusDefinition,
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
  project_id?: string | null;
}

function buildQuery(
  filters: IssueFilters,
  cursor?: string | null,
): Partial<SearchIssuesQuery> {
  const query: Partial<SearchIssuesQuery> = {
    limit: PAGE_SIZE,
  };
  // `SearchIssuesQuery.status` is now `string` (StatusKey) on the wire after
  // backend [[i-dlcqjubx]]; no cast required.
  if (filters.status) query.status = filters.status;
  if (filters.type) query.issue_type = filters.type as SearchIssuesQuery["issue_type"];
  if (filters.creator) query.creator = filters.creator;
  if (filters.assignee) query.assignee = filters.assignee;
  if (filters.labels) query.labels = filters.labels;
  if (filters.q) query.q = filters.q;
  if (filters.ids) query.ids = filters.ids;
  if (filters.project_id) query.project_id = filters.project_id;
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
 * Describes one project section on the board.
 */
export interface BoardProjectDescriptor {
  project_id: string;
  key: string;
  name: string;
  statuses: StatusDefinition[];
  default_status_key: string;
}

export interface BoardCellQuery {
  issues: IssueSummaryRecord[];
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  fetchNextPage: () => void;
}

/**
 * Result of `useBoardIssuesByProject`. Outer key is the project_id.
 * Inner key is the status key. Order of insertion mirrors the
 * `projects` argument for stable iteration.
 */
export type BoardCellsByProject = Map<string, Map<string, BoardCellQuery>>;

function cellKey(projectId: string, statusKey: string): string {
  return `${projectId}::${statusKey}`;
}

/**
 * Per-(project, status) paginated board hook. Fans out one query per cell
 * via `useQueries` so the column count is dynamic in both dimensions.
 *
 * Per-cell query key shares the `["paginatedIssues", …]` prefix used by the
 * table-view query so SSE invalidations propagate to both shapes.
 *
 * When the page's chip filter is active (`baseFilters.status` set), only the
 * column whose status matches the chip is queried; the other columns'
 * queries are disabled and render empty headers, matching the table-mode
 * semantics where a chip narrows to a single status.
 *
 * Pagination depth is tracked per cell as React state: `fetchNextPage` bumps
 * the depth, which re-keys the cell's query so `queryFn` walks one more page
 * through the server cursor chain. `keepPreviousData` keeps the prior pages
 * on screen while the deeper fetch lands, mirroring `useInfiniteQuery`'s
 * `isFetchingNextPage` UX.
 */
export function useBoardIssuesByProject(
  baseFilters: IssueFilters,
  projects: BoardProjectDescriptor[],
  enabled = true,
): BoardCellsByProject {
  const chipStatus = baseFilters.status ?? null;

  const [depthByCell, setDepthByCell] = useState<Record<string, number>>({});

  const cells = useMemo(() => {
    const out: Array<{ projectId: string; statusKey: string }> = [];
    for (const p of projects) {
      for (const s of p.statuses) {
        out.push({ projectId: p.project_id, statusKey: s.key });
      }
    }
    return out;
  }, [projects]);

  const queries = useQueries({
    queries: cells.map(({ projectId, statusKey }) => {
      const ck = cellKey(projectId, statusKey);
      const depth = depthByCell[ck] ?? 1;
      // When a chip filter is active and this column doesn't match it,
      // skip the fetch entirely — the cell renders as an empty header
      // and no network traffic is spent.
      const filteredOut = chipStatus !== null && chipStatus !== statusKey;
      const filtersForKey: IssueFilters = {
        ...baseFilters,
        project_id: projectId,
        status: statusKey,
      };
      return {
        // Depth suffix splits the cache per loaded-page-count so each
        // load-more is its own cache entry while still sharing the
        // `["paginatedIssues", …]` invalidation prefix.
        queryKey: ["paginatedIssues", filtersForKey, "depth", depth] as const,
        queryFn: async (): Promise<ListIssuesResponse[]> => {
          const pages: ListIssuesResponse[] = [];
          let cursor: string | undefined;
          for (let i = 0; i < depth; i++) {
            const page = await apiClient.listIssues(
              buildQuery(filtersForKey, cursor),
            );
            pages.push(page);
            if (!page.next_cursor) break;
            cursor = page.next_cursor;
          }
          return pages;
        },
        placeholderData: keepPreviousData,
        enabled: enabled && !filteredOut,
      };
    }),
  });

  const bumpDepth = useCallback(
    (projectId: string, statusKey: string) => {
      const ck = cellKey(projectId, statusKey);
      setDepthByCell((prev) => ({ ...prev, [ck]: (prev[ck] ?? 1) + 1 }));
    },
    [],
  );

  return useMemo(() => {
    const map: BoardCellsByProject = new Map();
    for (const p of projects) {
      map.set(p.project_id, new Map());
    }
    for (let i = 0; i < cells.length; i++) {
      const { projectId, statusKey } = cells[i];
      const query = queries[i];
      const filteredOut = chipStatus !== null && chipStatus !== statusKey;

      const pages = (query.data ?? []) as ListIssuesResponse[];
      const rawAll = pages.flatMap((p) => p.issues);
      const deduped = dedupeIssues(rawAll);
      const visible = filteredOut
        ? []
        : deduped.filter((rec) => rec.issue.status === statusKey);

      const lastPage = pages[pages.length - 1];
      const serverHasNext = !!lastPage?.next_cursor;
      // Approximate useInfiniteQuery's isFetchingNextPage: a depth bump
      // re-keys the query, React Query keeps the prior data via
      // placeholderData, and `isFetching` is true while the deeper fetch
      // runs.
      const isFetchingNext = query.isFetching && pages.length > 0;

      const cellQuery: BoardCellQuery = {
        issues: visible,
        isLoading: !filteredOut && query.isLoading,
        hasNextPage: !filteredOut && serverHasNext,
        isFetchingNextPage: !filteredOut && isFetchingNext,
        fetchNextPage: () => {
          if (serverHasNext && !isFetchingNext) {
            bumpDepth(projectId, statusKey);
          }
        },
      };
      map.get(projectId)!.set(statusKey, cellQuery);
    }
    return map;
  }, [projects, cells, queries, chipStatus, bumpDepth]);
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
      if (filters.status) query.status = filters.status;
      if (filters.type) query.issue_type = filters.type as SearchIssuesQuery["issue_type"];
      if (filters.creator) query.creator = filters.creator;
      if (filters.assignee) query.assignee = filters.assignee;
      if (filters.labels) query.labels = filters.labels;
      if (filters.q) query.q = filters.q;
      if (filters.ids) query.ids = filters.ids;
      if (filters.project_id) query.project_id = filters.project_id;
      const resp = await apiClient.listIssues(query);
      return Number(resp.total_count ?? 0);
    },
    placeholderData: keepPreviousData,
    enabled,
  });
}
