import { useQuery } from "@tanstack/react-query";
import type { PatchSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

export function usePatchesByIssue(patchIds: string[]) {
  const { data, isLoading, error } = useQuery({
    queryKey: ["patches", { ids: patchIds }],
    queryFn: () => apiClient.listPatches({ ids: patchIds.join(",") }),
    select: (resp): PatchSummaryRecord[] => resp.patches,
    enabled: patchIds.length > 0,
  });

  return { data: data ?? [], isLoading, error };
}
