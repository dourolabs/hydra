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

export function buildSections(
  issues: IssueSummaryRecord[],
  projects: ProjectRecord[] | undefined,
): { sections: ProjectSection[]; flat: boolean } {
  if (!projects || projects.length === 0) {
    return { sections: [], flat: true };
  }

  const projectById = new Map<string, ProjectRecord>();
  for (const p of projects) projectById.set(p.project_id, p);

  // Bucket issues by project_id while preserving first-occurrence order from
  // the server-ordered issues array. The list endpoint (PR-1) emits issues
  // sorted by (project.priority ASC, status.position ASC, created_at DESC,
  // id DESC) when `sort=project_status_time_desc`, so first-occurrence
  // iteration here yields sections in project.priority order with no
  // client-side reshuffle — and within each bucket the issues remain in the
  // status.position / created_at order the server produced.
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

  const sections: ProjectSection[] = [];
  for (const [key, bucket] of byProject) {
    const project = projectById.get(key);
    if (!project) continue;
    sections.push({
      groupKey: project.project_id,
      projectKey: project.project.key,
      projectName: project.project.name,
      statuses: project.project.statuses,
      issues: bucket,
    });
  }

  // Orphan bucket: any project_id that didn't resolve to a known project
  // (project was archived / not yet returned by useProjects()). Surface
  // those issues at the end rather than silently dropping them.
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
