import { useInfiniteQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function usePatches() {
  return useInfiniteQuery({
    queryKey: ["patches"],
    queryFn: ({ pageParam }) => apiClient.listPatches({ cursor: pageParam }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    select: (data) => data.pages.flatMap((page) => page.patches),
  });
}
