import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function usePatch(patchId: string) {
  return useQuery({
    queryKey: ["patch", patchId],
    queryFn: () => apiClient.getPatch(patchId),
    enabled: !!patchId,
  });
}
