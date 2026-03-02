import { useMemo } from "react";
import type { PatchSummaryRecord } from "@metis/api";
import { usePatches } from "./usePatches";

export function usePatchesByIssue(patchIds: string[]) {
  const { data: allPatches, isLoading, error } = usePatches();

  const data: PatchSummaryRecord[] = useMemo(() => {
    if (!allPatches) return [];
    const idSet = new Set(patchIds);
    return allPatches.filter((p) => idSet.has(p.patch_id));
  }, [allPatches, patchIds]);

  return { data, isLoading, error };
}
