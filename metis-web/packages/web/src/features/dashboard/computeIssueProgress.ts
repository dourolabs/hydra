import type { IssueSummaryRecord, JobSummaryRecord, SubtreeIssue } from "@metis/api";
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
 * An issue needs attention when it has a non-terminal status and matches the filter.
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

/**
 * Compute isActive map from server-provided subtree data.
 * An issue is "active" if it or any of its subtree descendants has an active task.
 */
export function computeIsActiveMapFromSubtree(
  issues: IssueSummaryRecord[],
): Map<string, boolean> {
  const cache = new Map<string, boolean>();

  function hasActiveDescendant(subtree: SubtreeIssue[]): boolean {
    for (const child of subtree) {
      if (child.has_active_task) return true;
      if (child.children && hasActiveDescendant(child.children)) return true;
    }
    return false;
  }

  for (const issue of issues) {
    const jobsSummary = issue.jobs_summary;
    const hasOwnActiveJob = !!(jobsSummary && jobsSummary.running > 0);
    const subtreeActive = issue.subtree ? hasActiveDescendant(issue.subtree) : false;
    cache.set(issue.issue_id, hasOwnActiveJob || subtreeActive);
  }
  return cache;
}

/**
 * Compute child status map from server-provided subtree data.
 * Maps each issue ID to an array of ChildStatus for its direct children.
 */
export function computeChildStatusFromSubtree(
  issues: IssueSummaryRecord[],
  username?: string,
): Map<string, ChildStatus[]> {
  const map = new Map<string, ChildStatus[]>();
  for (const issue of issues) {
    if (!issue.subtree || issue.subtree.length === 0) continue;
    const statuses: ChildStatus[] = issue.subtree.map((child) => {
      const childHasActive = child.has_active_task ||
        (child.children ? subtreeHasActive(child.children) : false);
      return {
        id: child.issue_id,
        status: child.status,
        hasActiveTask: childHasActive,
        assignedToUser: !!(username && child.assignee === username),
      };
    });
    map.set(issue.issue_id, statuses);
  }
  return map;
}

function subtreeHasActive(children: SubtreeIssue[]): boolean {
  for (const child of children) {
    if (child.has_active_task) return true;
    if (child.children && subtreeHasActive(child.children)) return true;
  }
  return false;
}
