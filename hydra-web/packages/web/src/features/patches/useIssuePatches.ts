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
  // Relations query is keyed directly by `issueId`; any key change is a
  // navigation, so don't `keepPreviousData` here.
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
  // Include `issueId` in the queryKey and gate `placeholderData` on it so we
  // only keep stale data for refetches within the same issue. The
  // ["patches"] prefix is preserved for SSE invalidation.
  const patchesQuery = useQuery({
    queryKey: ["patches", idsParam, issueId],
    queryFn: () => apiClient.listPatches({ ids: idsParam, limit: patchIds.length }),
    select: (resp): PatchSummaryRecord[] => resp.patches,
    enabled: patchIds.length > 0,
    staleTime: 30_000,
    placeholderData: (previousData, previousQuery) =>
      previousQuery?.queryKey[2] === issueId ? previousData : undefined,
  });

  const orderedPatches = useMemo(() => {
    if (patchIds.length === 0) return [];
    const map = new Map<string, PatchSummaryRecord>();
    for (const patch of patchesQuery.data ?? []) {
      map.set(patch.patch_id, patch);
    }
    const out: PatchSummaryRecord[] = [];
    for (const id of patchIds) {
      const patch = map.get(id);
      if (patch) out.push(patch);
    }
    return out;
  }, [patchIds, patchesQuery.data]);

  const isLoading =
    relationsQuery.isLoading || (patchIds.length > 0 && patchesQuery.isLoading);
  const error = relationsQuery.error ?? patchesQuery.error ?? null;

  return {
    data: orderedPatches,
    isLoading,
    error,
  };
}
