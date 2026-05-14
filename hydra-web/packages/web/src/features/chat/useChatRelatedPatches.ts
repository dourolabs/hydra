import { useQuery } from "@tanstack/react-query";
import type { PatchSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

const MAX_DISPLAYED = 25;

export interface RelatedPatchesResult {
  patches: PatchSummaryRecord[];
  isLoading: boolean;
}

/** Section 5: Most recent patches (capped at 25). */
export function useChatRelatedPatches(): RelatedPatchesResult {
  const query = useQuery({
    queryKey: ["chatRelated", "patches"],
    queryFn: () => apiClient.listPatches({ limit: MAX_DISPLAYED }),
    staleTime: 30_000,
    select: (data) => data.patches,
  });

  return {
    patches: query.data ?? [],
    isLoading: query.isLoading,
  };
}
