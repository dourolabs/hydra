import { keepPreviousData, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { SearchPatchesQuery, PatchStatus, ListPatchesResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

export interface PatchFilters {
  q?: string;
  status?: PatchStatus[];
}

export function usePaginatedPatches(filters: PatchFilters, enabled = true) {
  return useInfiniteQuery<ListPatchesResponse, Error>({
    queryKey: ["paginatedPatches", filters],
    queryFn: ({ pageParam }) => {
      const query: Partial<SearchPatchesQuery> = {
        limit: PAGE_SIZE,
      };
      if (filters.q) query.q = filters.q;
      if (filters.status && filters.status.length > 0) query.status = filters.status;
      if (pageParam) query.cursor = pageParam as string;
      return apiClient.listPatches(query);
    },
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
        limit: 0,
        count: true,
      };
      if (filters.q) query.q = filters.q;
      if (filters.status && filters.status.length > 0) query.status = filters.status;
      const resp = await apiClient.listPatches(query);
      return Number(resp.total_count ?? 0);
    },
    enabled,
  });
}
