import { keepPreviousData, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { SearchPatchesQuery, PatchStatus, ListPatchesResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

export interface PatchFilters {
  q?: string;
  status?: PatchStatus[];
  repo_name?: string;
  creator?: string;
  ids?: string;
}

function buildQuery(
  filters: PatchFilters,
  cursor?: string | null,
): Partial<SearchPatchesQuery> {
  const query: Partial<SearchPatchesQuery> = {
    limit: PAGE_SIZE,
  };
  if (filters.q) query.q = filters.q;
  if (filters.status && filters.status.length > 0) query.status = filters.status;
  if (filters.repo_name) query.repo_name = filters.repo_name;
  if (filters.creator) query.creator = filters.creator;
  if (filters.ids) query.ids = filters.ids;
  if (cursor) query.cursor = cursor;
  return query;
}

export function usePaginatedPatches(filters: PatchFilters, enabled = true) {
  return useInfiniteQuery<ListPatchesResponse, Error>({
    queryKey: ["paginatedPatches", filters],
    queryFn: ({ pageParam }) =>
      apiClient.listPatches(buildQuery(filters, pageParam as string | undefined)),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
}

/**
 * Count-only query for the page eyebrow total. Uses limit=0 and count=true
 * to get total_count without fetching patch data.
 */
export function usePatchCount(filters: PatchFilters, enabled = true) {
  return useQuery({
    queryKey: ["patchCount", filters],
    queryFn: async () => {
      const query: Partial<SearchPatchesQuery> = {
        ...buildQuery(filters),
        limit: 0,
        count: true,
      };
      const resp = await apiClient.listPatches(query);
      return Number(resp.total_count ?? 0);
    },
    placeholderData: keepPreviousData,
    enabled,
  });
}
