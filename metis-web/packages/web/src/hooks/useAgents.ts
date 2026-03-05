import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../api/client";

export function useAgents() {
  return useQuery({
    queryKey: ["agents"],
    queryFn: () => apiClient.listAgents(),
    select: (data) => data.agents,
  });
}
