import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";

export interface IssueProgress {
  rootId: string;
  rootIssue: IssueSummaryRecord;
  open: number;
  inProgress: number;
  closed: number;
  total: number;
  hasActive: boolean;
  needsAttentionCount: number;
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
      status === "open" &&
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
    };
  });
}
