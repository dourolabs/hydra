import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";

export function computeIsActiveMap(
  issues: IssueSummaryRecord[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): Map<string, boolean> {
  const childrenMap = new Map<string, string[]>();
  for (const issue of issues) {
    for (const dep of issue.issue.dependencies) {
      if (dep.type === "child-of") {
        const siblings = childrenMap.get(dep.issue_id) ?? [];
        siblings.push(issue.issue_id);
        childrenMap.set(dep.issue_id, siblings);
      }
    }
  }

  const cache = new Map<string, boolean>();

  function isActive(issueId: string): boolean {
    const cached = cache.get(issueId);
    if (cached !== undefined) return cached;
    const jobs = jobsByIssue.get(issueId) ?? [];
    if (jobs.some((j) => j.task.status === "running" || j.task.status === "pending")) {
      cache.set(issueId, true);
      return true;
    }
    const children = childrenMap.get(issueId) ?? [];
    const result = children.some((childId) => isActive(childId));
    cache.set(issueId, result);
    return result;
  }

  for (const issue of issues) {
    isActive(issue.issue_id);
  }
  return cache;
}

export interface ChildStatus {
  id: string;
  status: string;
  hasActiveTask: boolean;
  assignedToUser: boolean;
}

export interface IssueProgress {
  rootId: string;
  rootIssue: IssueSummaryRecord;
  open: number;
  inProgress: number;
  closed: number;
  total: number;
  hasActive: boolean;
  needsAttentionCount: number;
  children: ChildStatus[];
}

/**
 * Count issues needing attention for a badge.
 * An issue needs attention when it is open/in-progress and matches the filter.
 */
export function countNeedsAttentionBadge(
  issues: IssueSummaryRecord[],
  filter: (issue: IssueSummaryRecord) => boolean,
  isActiveMap?: Map<string, boolean>,
): number {
  return issues.filter((issue) => {
    const status = issue.issue.status;
    if (TERMINAL_STATUSES.has(status)) return false;
    if (isActiveMap?.get(issue.issue_id)) return false;
    return filter(issue);
  }).length;
}

export function computeIssueProgress(
  roots: IssueTreeNode[],
  jobsByIssue?: Map<string, JobSummaryRecord[]>,
  username?: string,
): IssueProgress[] {
  function hasActiveDescendant(node: IssueTreeNode): boolean {
    if (!jobsByIssue) return false;
    const jobs = jobsByIssue.get(node.id) ?? [];
    if (jobs.some((j) => j.task.status === "running" || j.task.status === "pending")) {
      return true;
    }
    return node.children.some((child) => hasActiveDescendant(child));
  }

  function countNeedsAttention(node: IssueTreeNode): number {
    let count = 0;
    const status = node.issue.issue.status;
    if (
      username &&
      (status === "open" || status === "in-progress") &&
      node.issue.issue.assignee === username
    ) {
      const jobs = jobsByIssue?.get(node.id) ?? [];
      const hasRunningJob = jobs.some(
        (j) => j.task.status === "running" || j.task.status === "pending",
      );
      if (!hasRunningJob) {
        count++;
      }
    }
    for (const child of node.children) {
      count += countNeedsAttention(child);
    }
    return count;
  }

  return roots.map((root) => {
    let open = 0;
    let inProgress = 0;
    let closed = 0;
    const childStatuses: ChildStatus[] = [];

    for (const child of root.children) {
      if (child.hardBlocked) continue;
      const status = child.issue.issue.status;
      if (status === "closed") {
        closed++;
      } else if (status === "in-progress") {
        inProgress++;
      } else if (status === "open") {
        open++;
      }

      const hasActiveTask = hasActiveDescendant(child);
      const assignedToUser = !!(
        username &&
        child.issue.issue.assignee === username
      );
      childStatuses.push({
        id: child.id,
        status,
        hasActiveTask,
        assignedToUser,
      });
    }

    return {
      rootId: root.id,
      rootIssue: root.issue,
      open,
      inProgress,
      closed,
      total: open + inProgress + closed,
      hasActive: hasActiveDescendant(root),
      needsAttentionCount: countNeedsAttention(root),
      children: childStatuses,
    };
  });
}
