import type {
  ListProjectsResponse,
  Project,
  ProjectId,
  ProjectRecord,
} from "@hydra/api";

export const PROJECTS_QUERY_KEY = ["projects"] as const;

export function applyOptimisticUpsert(
  list: ProjectRecord[],
  projectId: ProjectId | null,
  project: Project,
): ProjectRecord[] {
  if (projectId) {
    return list.map((rec) =>
      rec.project_id === projectId
        ? { ...rec, project, version: rec.version + 1 }
        : rec,
    );
  }
  const placeholder: ProjectRecord = {
    project_id: `optimistic:${project.key}`,
    version: 1,
    project,
  };
  return [...list, placeholder];
}

export function applyOptimisticDelete(
  list: ProjectRecord[],
  projectId: ProjectId,
): ProjectRecord[] {
  return list.filter((rec) => rec.project_id !== projectId);
}

export type ProjectsCacheSnapshot = ListProjectsResponse | undefined;
