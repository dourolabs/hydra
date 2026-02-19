import { useQuery } from "@tanstack/react-query";
import { apiFetch } from "./client";

export interface RepositoryRecord {
  name: string;
  repository: {
    remote_url?: string;
    default_branch?: string;
  };
}

export interface ListRepositoriesResponse {
  repositories: RepositoryRecord[];
}

function fetchRepositories(): Promise<ListRepositoriesResponse> {
  return apiFetch<ListRepositoriesResponse>("/api/v1/repositories");
}

export function useRepositories() {
  return useQuery({
    queryKey: ["repositories"],
    queryFn: fetchRepositories,
    select: (data) => data.repositories,
  });
}
