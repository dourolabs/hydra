import { useInfiniteQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useDocuments() {
  return useInfiniteQuery({
    queryKey: ["documents"],
    queryFn: ({ pageParam }) => apiClient.listDocuments({ cursor: pageParam }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    select: (data) => data.pages.flatMap((page) => page.documents),
  });
}
