import type {
  IssueSummaryRecord,
  ProjectRecord,
  StatusDefinition,
} from "@hydra/api";

// Sentinel group key for issues whose `project_id` doesn't resolve to a
// known project (e.g. references a project not returned by useProjects()).
// Real project ids never collide with this string.
export const UNRESOLVED_GROUP_KEY = "__unresolved__";

export interface ProjectSection {
  groupKey: string;
  projectKey: string;
  projectName: string | null;
  statuses: StatusDefinition[];
  issues: IssueSummaryRecord[];
}

export function findDefaultProject(
  projects: ProjectRecord[] | undefined,
): ProjectRecord | null {
  if (!projects || projects.length === 0) return null;
  return projects.find((p) => p.project.key === "default") ?? null;
}

export function buildSections(
  issues: IssueSummaryRecord[],
  projects: ProjectRecord[] | undefined,
): { sections: ProjectSection[]; flat: boolean } {
  if (!projects || projects.length === 0) {
    return { sections: [], flat: true };
  }
  const defaultProject = findDefaultProject(projects);

  const byProject = new Map<string, IssueSummaryRecord[]>();
  for (const rec of issues) {
    const key = rec.issue.project_id;
    const bucket = byProject.get(key);
    if (bucket) {
      bucket.push(rec);
    } else {
      byProject.set(key, [rec]);
    }
  }

  const ordered: ProjectRecord[] = [];
  if (defaultProject) ordered.push(defaultProject);
  for (const p of projects) {
    if (p === defaultProject) continue;
    ordered.push(p);
  }

  const sections: ProjectSection[] = [];
  for (const project of ordered) {
    const bucket = byProject.get(project.project_id);
    if (!bucket || bucket.length === 0) continue;
    sections.push({
      groupKey: project.project_id,
      projectKey: project.project.key,
      projectName: project.project.name,
      statuses: project.project.statuses,
      issues: bucket,
    });
  }

  for (const [key, bucket] of byProject) {
    if (sections.some((s) => s.groupKey === key)) continue;
    sections.push({
      groupKey: key,
      projectKey: key === UNRESOLVED_GROUP_KEY ? "unknown" : key,
      projectName: null,
      statuses: [],
      issues: bucket,
    });
  }

  return { sections, flat: false };
}
