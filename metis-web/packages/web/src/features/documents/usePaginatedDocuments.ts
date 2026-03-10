import { useInfiniteQuery } from "@tanstack/react-query";
import type { ListDocumentsResponse } from "@metis/api";
import { apiClient } from "../../api/client";

const DEFAULT_PAGE_SIZE = 50;

interface PaginatedDocumentsOptions {
  limit?: number;
}

/**
 * Fetches documents page-by-page using cursor-based pagination.
 */
export function usePaginatedDocuments(options?: PaginatedDocumentsOptions) {
  const limit = options?.limit ?? DEFAULT_PAGE_SIZE;

  return useInfiniteQuery<ListDocumentsResponse, Error>({
    queryKey: ["paginatedDocuments"],
    queryFn: ({ pageParam }) =>
      apiClient.listDocuments({
        limit,
        cursor: (pageParam as string) ?? null,
      }),
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    initialPageParam: null as string | null,
    staleTime: 30_000,
  });
}
