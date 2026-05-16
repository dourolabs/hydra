import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { PatchSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Fetch patches linked to an issue via the `has-patch` relation.
 * Queries relations first, then batch-fetches the patch summaries.
 *
 * The relations query key is shaped so the SSE `['relations', 'has-patch']`
 * invalidation in useSSE refreshes it.
 */
export function useIssuePatches(issueId: string) {
  const relationsQuery = useQuery({
    queryKey: ["relations", "has-patch", issueId],
    queryFn: () =>
      apiClient.listRelations({
        source_id: issueId,
        rel_type: "has-patch",
      }),
    enabled: !!issueId,
    staleTime: 30_000,
    select: (data) => data.relations,
  });

  const patchIds = useMemo(
    () => relationsQuery.data?.map((rel) => rel.target_id) ?? [],
    [relationsQuery.data],
  );

  const idsParam = patchIds.join(",");
  const patchesQuery = useQuery({
    queryKey: ["patches", { ids: patchIds }],
    queryFn: () => apiClient.listPatches({ ids: idsParam }),
    select: (resp): PatchSummaryRecord[] => resp.patches,
    enabled: patchIds.length > 0,
    staleTime: 30_000,
  });

  const isLoading =
    relationsQuery.isLoading || (patchIds.length > 0 && patchesQuery.isLoading);
  const error = relationsQuery.error ?? patchesQuery.error ?? null;

  return {
    data: patchIds.length === 0 ? [] : (patchesQuery.data ?? []),
    isLoading,
    error,
  };
}
