import type { IssueSummaryRecord, SessionSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";

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

export function computeIssueProgress(
  roots: IssueTreeNode[],
  sessionsByIssue?: Map<string, SessionSummaryRecord[]>,
  username?: string,
): IssueProgress[] {
  function hasActiveDescendant(node: IssueTreeNode): boolean {
    if (!sessionsByIssue) return false;
    const jobs = sessionsByIssue.get(node.id) ?? [];
    if (jobs.some((j) => j.session.status === "running" || j.session.status === "pending")) {
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
      const jobs = sessionsByIssue?.get(node.id) ?? [];
      const hasRunningJob = jobs.some(
        (j) => j.session.status === "running" || j.session.status === "pending",
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
