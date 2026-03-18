import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../api/client";

export function useRepositories() {
  return useQuery({
    queryKey: ["repositories"],
    queryFn: () => apiClient.listRepositories(),
    select: (data) => data.repositories,
  });
}
