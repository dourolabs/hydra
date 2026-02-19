import { useQuery } from "@tanstack/react-query";
import type { RepositoryRecord, ListRepositoriesResponse } from "@metis/api";
import { apiClient } from "./client";

export type { RepositoryRecord, ListRepositoriesResponse };

function fetchRepositories(): Promise<ListRepositoriesResponse> {
  return apiClient.listRepositories();
}

export function useRepositories() {
  return useQuery({
    queryKey: ["repositories"],
    queryFn: fetchRepositories,
    select: (data) => data.repositories,
  });
}
