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

// Sentinel marker used in the board bulk query key so the optimistic
// drag-drop walk in `IssuesBoard.tsx` can disambiguate the bulk shape
// (single `ListIssuesResponse`) from the per-cell expanded shape
// (`ListIssuesResponse[]`, keyed with `"depth"`) and the table-view
// infinite-query shape ({ pages, pageParams }, keyed with `"sort"`).
export const BOARD_BULK_QUERY_KEY_MARKER = "board-bulk";

const PAGE_SIZE = 50;
// The board view packs many (project × status) cells onto one screen, so it
// loads a much shorter first page per cell to save real estate. "Load more"
// then walks the cursor a page at a time. The table view keeps PAGE_SIZE.
const BOARD_PAGE_SIZE = 7;

export interface IssueFilters {
  status?: string | null;
  type?: string | null;
  creator?: string | null;
  assignee?: string | null;
  labels?: string | null;
  q?: string | null;
  ids?: string | null;
  project_id?: string | null;
  /**
   * When true, the backend includes soft-deleted (archived) issues in the
   * result set. Maps to `?include_archived=true` on the issues list endpoint.
   */
  include_archived?: boolean | null;
}

function buildQuery(
  filters: IssueFilters,
  cursor?: string | null,
  limit?: number,
  sort?: SearchIssuesQuery["sort"],
): Partial<SearchIssuesQuery> {
  const query: Partial<SearchIssuesQuery> = {};
  if (limit !== undefined) query.limit = limit;
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
  if (filters.include_archived) query.include_archived = true;
  if (cursor) query.cursor = cursor;
  if (sort) query.sort = sort;
  return query;
}

// The Issues list page renders sections in project order (PR-1 + PR-2):
// the server emits issues already sorted by
// (project.priority ASC, status.position ASC, created_at DESC, id DESC),
// and the renderer in `projectSections.buildSections` groups by `project_id`
// in first-occurrence order. The count query intentionally stays on the
// default sort.
const LIST_PAGE_SORT: SearchIssuesQuery["sort"] = "project_status_time_desc";

// Board sort: same vocabulary as the list page. With `bucket_by=project_status`,
// `project.priority` and `status.position` are pinned within each cell, so the
// effective within-cell order is `created_at DESC, id DESC`.
const BOARD_SORT: SearchIssuesQuery["sort"] = "project_status_time_desc";

/**
 * Paginated issues hook using cursor-based pagination with React Query's
 * useInfiniteQuery. Query key includes all active filters so changing
 * filters automatically refetches.
 */
export function usePaginatedIssues(filters: IssueFilters, enabled = true) {
  return useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: ["paginatedIssues", filters, "sort", LIST_PAGE_SORT],
    queryFn: ({ pageParam }) =>
      apiClient.listIssues(
        buildQuery(
          filters,
          pageParam as string | undefined,
          PAGE_SIZE,
          LIST_PAGE_SORT,
        ),
      ),
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
 * Per-(project, status) paginated board hook. Issues a single bucketed
 * `GET /v1/issues` call returning the top `BOARD_PAGE_SIZE` issues per
 * `(project_id, status.key)` cell, then groups the response client-side
 * into the `BoardCellsByProject` shape.
 *
 * The bulk query key shares the `["paginatedIssues", …]` prefix used by
 * the table-view and per-cell queries so SSE invalidations propagate to
 * all shapes.
 *
 * Per-cell "Load more": when a cell's `fetchNextPage` is called, that
 * cell flips to a single-cell unbucketed `useQueries` entry that walks
 * the server cursor chain at `BOARD_PAGE_SIZE` per page. Non-expanded
 * cells continue to render from the bulk query's bucket. Expansion is
 * tracked as a per-cell React state map (`expandedDepth`).
 *
 * The chip-filter case (`baseFilters.status` set) keeps the bulk query
 * path — bucket_by=project_status with a single-status filter degenerates
 * to "top N per project" without changing the request shape.
 */
export function useBoardIssuesByProject(
  baseFilters: IssueFilters,
  projects: BoardProjectDescriptor[],
  enabled = true,
): BoardCellsByProject {
  const chipStatus = baseFilters.status ?? null;

  // expandedDepth[ck]: number of pages walked by the per-cell unbucketed
  // query. 0 means not expanded — the cell renders from the bulk query.
  // First click bumps to 2 so the user sees ~2× the bulk-bucket view
  // (depth=1 would refetch the same first page and look like nothing
  // happened).
  const [expandedDepth, setExpandedDepth] = useState<Record<string, number>>(
    {},
  );

  const cells = useMemo(() => {
    const out: Array<{ projectId: string; statusKey: string }> = [];
    for (const p of projects) {
      for (const s of p.statuses) {
        out.push({ projectId: p.project_id, statusKey: s.key });
      }
    }
    return out;
  }, [projects]);

  // One bulk bucketed request collapses the historical N-query fan-out
  // (10 projects × 5–8 statuses ⇒ 50–80 requests) into a single roundtrip.
  // The response is grouped client-side into per-cell buckets below.
  const bulkQuery = useQuery({
    queryKey: [
      "paginatedIssues",
      baseFilters,
      BOARD_BULK_QUERY_KEY_MARKER,
      BOARD_SORT,
    ] as const,
    queryFn: () =>
      apiClient.listIssues({
        ...buildQuery(baseFilters, undefined, undefined, BOARD_SORT),
        bucket_by: "project_status",
        bucket_limit: BOARD_PAGE_SIZE,
      }),
    placeholderData: keepPreviousData,
    enabled,
  });

  // Per-expanded-cell single-cell unbucketed paginated queries. Disabled
  // until the user clicks "Load more" on the cell (depth === 0 keeps the
  // bulk-bucket view; depth ≥ 1 spawns the cell's own cursor walk).
  const expandedQueries = useQueries({
    queries: cells.map(({ projectId, statusKey }) => {
      const ck = cellKey(projectId, statusKey);
      const depth = expandedDepth[ck] ?? 0;
      // Chip-filtered columns that don't match the chip stay empty and
      // never expand — no need to keep a disabled query entry around.
      const filteredOut = chipStatus !== null && chipStatus !== statusKey;
      const filtersForKey: IssueFilters = {
        ...baseFilters,
        project_id: projectId,
        status: statusKey,
      };
      return {
        // Depth suffix splits the cache per loaded-page-count, mirroring
        // the previous fan-out hook's per-cell shape so the drag-drop
        // optimistic update in `IssuesBoard.tsx` keeps working.
        queryKey: ["paginatedIssues", filtersForKey, "depth", depth] as const,
        queryFn: async (): Promise<ListIssuesResponse[]> => {
          const pages: ListIssuesResponse[] = [];
          let cursor: string | undefined;
          for (let i = 0; i < depth; i++) {
            const page = await apiClient.listIssues(
              buildQuery(filtersForKey, cursor, BOARD_PAGE_SIZE, BOARD_SORT),
            );
            pages.push(page);
            if (!page.next_cursor) break;
            cursor = page.next_cursor;
          }
          return pages;
        },
        placeholderData: keepPreviousData,
        enabled: enabled && !filteredOut && depth > 0,
      };
    }),
  });

  // Group the bulk response by `(project_id, status.key)` for cells that
  // haven't been expanded into their own query. The response preserves the
  // global sort across cells (PR-3 spec), so the first record encountered
  // for each cell is the within-cell top.
  const bulkByCell = useMemo(() => {
    const map = new Map<string, IssueSummaryRecord[]>();
    const issues = bulkQuery.data?.issues ?? [];
    for (const rec of issues) {
      const projectId = rec.issue.project_id ?? "";
      const statusKey = rec.issue.status.key;
      const ck = cellKey(projectId, statusKey);
      const arr = map.get(ck);
      if (arr) {
        arr.push(rec);
      } else {
        map.set(ck, [rec]);
      }
    }
    return map;
  }, [bulkQuery.data]);

  const bumpDepth = useCallback(
    (projectId: string, statusKey: string) => {
      const ck = cellKey(projectId, statusKey);
      setExpandedDepth((prev) => {
        const current = prev[ck] ?? 0;
        const next = current === 0 ? 2 : current + 1;
        return { ...prev, [ck]: next };
      });
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
      const ck = cellKey(projectId, statusKey);
      const filteredOut = chipStatus !== null && chipStatus !== statusKey;
      const depth = expandedDepth[ck] ?? 0;
      const isExpanded = depth > 0;

      const expQuery = expandedQueries[i];
      const expPages = (expQuery.data ?? []) as ListIssuesResponse[];
      const expIssues = isExpanded
        ? dedupeIssues(expPages.flatMap((p) => p.issues)).filter(
            (rec) => rec.issue.status.key === statusKey,
          )
        : [];

      const bulkIssues = bulkByCell.get(ck) ?? [];

      let issues: IssueSummaryRecord[];
      let hasNextPage: boolean;
      let isFetchingNext: boolean;
      let isLoading: boolean;

      if (filteredOut) {
        issues = [];
        hasNextPage = false;
        isFetchingNext = false;
        isLoading = false;
      } else if (isExpanded) {
        // Fall back to the bulk-bucket view while the expanded query is
        // still in flight on its first depth bump, so the cell never goes
        // blank between the two query shapes.
        issues = expPages.length > 0 ? expIssues : bulkIssues;
        const lastPage = expPages[expPages.length - 1];
        hasNextPage = !!lastPage?.next_cursor;
        isFetchingNext = expQuery.isFetching;
        isLoading = expQuery.isLoading && bulkQuery.isLoading;
      } else {
        issues = bulkIssues;
        // The bucketed response has no per-cell cursor (`next_cursor: null`
        // per PR-3). Heuristic: a cell that hit `bucket_limit` *may* have
        // more on the server; expand it via the single-cell unbucketed
        // query path to find out.
        hasNextPage = bulkIssues.length >= BOARD_PAGE_SIZE;
        isFetchingNext = false;
        isLoading = bulkQuery.isLoading;
      }

      const cellQuery: BoardCellQuery = {
        issues,
        isLoading,
        hasNextPage,
        isFetchingNextPage: isFetchingNext,
        fetchNextPage: () => {
          if (!hasNextPage || isFetchingNext) return;
          bumpDepth(projectId, statusKey);
        },
      };
      map.get(projectId)!.set(statusKey, cellQuery);
    }
    return map;
  }, [
    projects,
    cells,
    bulkByCell,
    expandedQueries,
    expandedDepth,
    chipStatus,
    bumpDepth,
    bulkQuery.isLoading,
  ]);
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
      if (filters.include_archived) query.include_archived = true;
      const resp = await apiClient.listIssues(query);
      return Number(resp.total_count ?? 0);
    },
    placeholderData: keepPreviousData,
    enabled,
  });
}
