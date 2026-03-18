import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import type { PatchVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Fetch patches by their IDs using individual queries.
 * Each patch is fetched and cached independently via its ["patch", id] query key,
 * which SSE events already invalidate for real-time updates.
 */
export function usePatchesByIds(patchIds: string[]) {
  const stableIds = useMemo(() => [...patchIds].sort(), [patchIds]);

  const queries = useQueries({
    queries: stableIds.map((id) => ({
      queryKey: ["patch", id],
      queryFn: () => apiClient.getPatch(id),
      staleTime: 30_000,
      enabled: !!id,
    })),
  });

  const data: PatchVersionRecord[] = useMemo(() => {
    const results: PatchVersionRecord[] = [];
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
