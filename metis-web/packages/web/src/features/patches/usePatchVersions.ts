import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function usePatchVersions(patchId: string) {
  return useQuery({
    queryKey: ["patch", patchId, "versions"],
    queryFn: () => apiClient.listPatchVersions(patchId),
    enabled: !!patchId,
  });
}
