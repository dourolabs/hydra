import { useQuery } from "@tanstack/react-query";
import type { SearchDocumentsQuery } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Count-only query for the Documents page eyebrow total. Uses `limit=0` and
 * `count=true` so the response carries `total_count` without fetching any
 * document rows. The backend's default behavior excludes soft-deleted
 * documents (`include_deleted` defaults to false), which matches the rendered
 * tree's visible set.
 */
export function useDocumentCount(enabled = true) {
  return useQuery({
    queryKey: ["documentCount"],
    queryFn: async () => {
      const query: Partial<SearchDocumentsQuery> = {
        limit: 0,
        count: true,
      };
      const resp = await apiClient.listDocuments(query);
      return Number(resp.total_count ?? 0);
    },
    enabled,
  });
}
