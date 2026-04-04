import { keepPreviousData, useInfiniteQuery } from "@tanstack/react-query";
import type { SearchDocumentsQuery, ListDocumentsResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

export function usePaginatedDocuments(searchQuery: string, enabled = true) {
  return useInfiniteQuery<ListDocumentsResponse, Error>({
    queryKey: ["paginatedDocuments", searchQuery],
    queryFn: ({ pageParam }) => {
      const query: Partial<SearchDocumentsQuery> = {
        limit: PAGE_SIZE,
      };
      if (searchQuery) query.q = searchQuery;
      if (pageParam) query.cursor = pageParam as string;
      return apiClient.listDocuments(query);
    },
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    enabled,
  });
}
