import { keepPreviousData, useInfiniteQuery } from "@tanstack/react-query";
import type { SearchPatchesQuery, ListPatchesResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

export function usePaginatedPatches(searchQuery: string, enabled = true) {
  return useInfiniteQuery<ListPatchesResponse, Error>({
    queryKey: ["paginatedPatches", searchQuery],
    queryFn: ({ pageParam }) => {
      const query: Partial<SearchPatchesQuery> = {
        limit: PAGE_SIZE,
      };
      if (searchQuery) query.q = searchQuery;
      if (pageParam) query.cursor = pageParam as string;
      return apiClient.listPatches(query);
    },
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
}
