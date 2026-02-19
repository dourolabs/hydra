import { useQueries } from "@tanstack/react-query";
import type { PatchVersionRecord } from "@metis/api";
import { apiClient } from "../../api/client";

export function usePatchesByIssue(patchIds: string[]) {
  const queries = useQueries({
    queries: patchIds.map((patchId) => ({
      queryKey: ["patch", patchId],
      queryFn: () => apiClient.getPatch(patchId),
      enabled: !!patchId,
    })),
  });

  const isLoading = queries.some((q) => q.isLoading);
  const error = queries.find((q) => q.error)?.error ?? null;
  const data: PatchVersionRecord[] = queries
    .map((q) => q.data)
    .filter((d): d is PatchVersionRecord => d !== undefined);

  return { data, isLoading, error };
}
