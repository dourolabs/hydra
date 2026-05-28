import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useLabel(labelId: string) {
  return useQuery({
    queryKey: ["label", labelId],
    queryFn: () => apiClient.getLabel(labelId),
    enabled: !!labelId,
  });
}
