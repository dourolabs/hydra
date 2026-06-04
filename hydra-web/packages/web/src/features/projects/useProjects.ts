import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

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
 * Fetch the status list for a project (or the synthesized default
 * project when `projectId` is null). Cached per project for the session
 * via React Query's default staleTime semantics.
 *
 * The status-update modal and any other status-picker UI is the only
 * project-aware frontend code that calls this. Issue badges read
 * `issue.resolved_status` directly and do NOT call this hook — keeping
 * resolution server-side per `/designs/per-project-issue-statuses.md`
 * §6 "Code cleanliness".
 */
export function useProjectStatuses(projectId: string | null | undefined) {
  return useQuery({
    queryKey: ["project-statuses", projectId ?? "default"],
    queryFn: () =>
      projectId
        ? apiClient.getProjectStatuses(projectId)
        : apiClient.getDefaultProjectStatuses(),
  });
}
