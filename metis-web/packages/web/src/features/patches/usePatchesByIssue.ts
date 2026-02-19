import { useQueries } from "@tanstack/react-query";
import { fetchPatch, type Patch } from "../../api/patches";

export function usePatchesByIssue(patchIds: string[]) {
  const queries = useQueries({
    queries: patchIds.map((patchId) => ({
      queryKey: ["patch", patchId],
      queryFn: () => fetchPatch(patchId),
      enabled: !!patchId,
    })),
  });

  const isLoading = queries.some((q) => q.isLoading);
  const error = queries.find((q) => q.error)?.error ?? null;
  const data: Patch[] = queries
    .map((q) => q.data)
    .filter((d): d is Patch => d !== undefined);

  return { data, isLoading, error };
}
