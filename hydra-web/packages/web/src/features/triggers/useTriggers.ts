import { useQuery } from "@tanstack/react-query";
import type { SearchTriggersQuery } from "@hydra/api";
import { apiClient } from "../../api/client";

export function useTriggers(query?: Partial<SearchTriggersQuery>) {
  return useQuery({
    queryKey: ["triggers", query],
    queryFn: () => apiClient.listTriggers(query),
    select: (data) => data.triggers,
  });
}

export function useTrigger(triggerId: string) {
  return useQuery({
    queryKey: ["trigger", triggerId],
    queryFn: () => apiClient.getTrigger(triggerId),
    enabled: !!triggerId,
  });
}
