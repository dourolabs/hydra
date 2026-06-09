import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

/** Seeded default project id. Mirrors `ProjectId::default_project()` on the server. */
const DEFAULT_PROJECT_ID = "j-defaul";

export function useProjects() {
  return useQuery({
    queryKey: ["projects"],
    queryFn: () => apiClient.listProjects(),
    select: (data) => data.projects.filter((p) => !p.project.deleted),
  });
}

export function useProject(projectId: string | null) {
  return useQuery({
    queryKey: ["project", projectId],
    queryFn: () => apiClient.getProject(projectId!),
    enabled: !!projectId,
  });
}

/**
 * Fetch the status list for a project (or the seeded default project
 * when `projectId` is null). Cached per project for the session via
 * React Query's default staleTime semantics.
 */
export function useProjectStatuses(projectId: string | null | undefined) {
  const resolved = projectId ?? DEFAULT_PROJECT_ID;
  return useQuery({
    queryKey: ["project-statuses", resolved],
    queryFn: () => apiClient.getProjectStatuses(resolved),
  });
}
