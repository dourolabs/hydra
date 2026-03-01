import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";

/**
 * Progress stats for a root issue's direct children.
 * Only tracks open/in-progress/closed; failed/dropped/rejected are excluded.
 */
export interface IssueProgress {
  rootId: string;
  rootIssue: IssueSummaryRecord;
  open: number;
  inProgress: number;
  closed: number;
  total: number;
  /** True if any descendant (recursive) has a running or pending job. */
  hasActive: boolean;
  /** Count of descendants that need attention (open, assigned to user, no active job). */
  needsAttentionCount: number;
}

/**
 * Activity section for a subtask in the unified activity feed.
 */
export type ActivitySection =
  | "active"
  | "needs-attention"
  | "upcoming"
  | "recently-completed";

export interface ActivityItem {
  issueId: string;
  issue: IssueSummaryRecord;
  parentIssueId: string;
  parentDescription: string;
  section: ActivitySection;
  /** Running/pending job for "active" items */
  activeJob?: JobSummaryRecord;
  /** Timestamp used for sorting within a section */
  sortTime: string;
}

export interface ActivitySummary {
  activeCount: number;
  needsAttentionCount: number;
  completedCount: number;
  totalCount: number;
}

/**
 * Classify a subtask into an activity section.
 *
 * 1. Running/pending job → "active"
 * 2. Terminal status (closed/failed/rejected/dropped) → "recently-completed"
 * 3. Open + assigned to current user → "needs-attention"
 * 4. Otherwise → "upcoming"
 */
export function classifyActivity(
  node: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
  username: string,
): ActivitySection {
  const jobs = jobsByIssue.get(node.id) ?? [];
  const hasRunningJob = jobs.some(
    (j) => j.task.status === "running" || j.task.status === "pending",
  );

  if (hasRunningJob) return "active";

  const status = node.issue.issue.status;
  if (TERMINAL_STATUSES.has(status)) return "recently-completed";

  if (status === "open" && node.issue.issue.assignee === username) {
    return "needs-attention";
  }

  return "upcoming";
}

/**
 * Collect activity items from all root issues created by the user.
 *
 * For each root: if it has children, collect the children (recursively).
 * If a root has no children, include the root itself.
 * Skips hardBlocked nodes.
 */
export function collectActivityItems(
  roots: IssueTreeNode[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
  username: string,
): ActivityItem[] {
  const items: ActivityItem[] = [];

  function walkChildren(
    node: IssueTreeNode,
    parentId: string,
    parentDesc: string,
  ) {
    if (node.hardBlocked) return;

    const section = classifyActivity(node, jobsByIssue, username);
    const jobs = jobsByIssue.get(node.id) ?? [];
    const activeJob = jobs.find(
      (j) => j.task.status === "running" || j.task.status === "pending",
    );

    items.push({
      issueId: node.id,
      issue: node.issue,
      parentIssueId: parentId,
      parentDescription: parentDesc,
      section,
      activeJob,
      sortTime: activeJob?.task.start_time ?? node.issue.timestamp,
    });

    for (const child of node.children) {
      walkChildren(child, parentId, parentDesc);
    }
  }

  for (const root of roots) {
    const rootDesc = root.issue.issue.description.split("\n")[0].trim();
    if (root.children.length === 0) {
      // Root has no children — include itself
      walkChildren(root, root.id, rootDesc);
    } else {
      for (const child of root.children) {
        walkChildren(child, root.id, rootDesc);
      }
    }
  }

  return items;
}

/**
 * Sort activity items: active first, then needs-attention, upcoming,
 * recently-completed. Within each section, most recent first.
 */
export function sortActivityItems(items: ActivityItem[]): ActivityItem[] {
  const priority: Record<ActivitySection, number> = {
    active: 0,
    "needs-attention": 1,
    upcoming: 2,
    "recently-completed": 3,
  };
  return [...items].sort((a, b) => {
    const p = priority[a.section] - priority[b.section];
    if (p !== 0) return p;
    return new Date(b.sortTime).getTime() - new Date(a.sortTime).getTime();
  });
}

/**
 * Compute summary counts for the summary bar.
 */
export function computeSummary(items: ActivityItem[]): ActivitySummary {
  let activeCount = 0;
  let needsAttentionCount = 0;
  let completedCount = 0;

  for (const item of items) {
    switch (item.section) {
      case "active":
        activeCount++;
        break;
      case "needs-attention":
        needsAttentionCount++;
        break;
      case "recently-completed":
        completedCount++;
        break;
    }
  }

  return {
    activeCount,
    needsAttentionCount,
    completedCount,
    totalCount: items.length,
  };
}

/**
 * Section display label.
 */
export function sectionLabel(section: ActivitySection): string {
  switch (section) {
    case "active":
      return "ACTIVE";
    case "needs-attention":
      return "NEEDS ATTENTION";
    case "upcoming":
      return "UPCOMING";
    case "recently-completed":
      return "COMPLETED";
  }
}

/**
 * Compute progress stats for each root issue by iterating only over
 * 1-level deep children (direct children, NOT recursive).
 *
 * Only counts open / in-progress / closed statuses.
 * Skips hardBlocked children and children with status failed/dropped/rejected.
 */
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
      // failed/dropped/rejected are intentionally skipped
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
