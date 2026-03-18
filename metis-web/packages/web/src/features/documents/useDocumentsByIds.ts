import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import type { DocumentVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Fetch documents by their IDs using individual queries.
 * Each document is fetched and cached independently via its ["document", id] query key,
 * which SSE events already invalidate for real-time updates.
 */
export function useDocumentsByIds(documentIds: string[]) {
  const stableIds = useMemo(() => [...documentIds].sort(), [documentIds]);

  const queries = useQueries({
    queries: stableIds.map((id) => ({
      queryKey: ["document", id],
      queryFn: () => apiClient.getDocument(id),
      staleTime: 30_000,
      enabled: !!id,
    })),
  });

  const data: DocumentVersionRecord[] = useMemo(() => {
    const results: DocumentVersionRecord[] = [];
    for (const q of queries) {
      if (q.data) {
        results.push(q.data);
      }
    }
    return results;
  }, [queries]);

  const isLoading = queries.some((q) => q.isLoading);
  const error = queries.find((q) => q.error)?.error ?? null;

  return { data, isLoading, error };
}
