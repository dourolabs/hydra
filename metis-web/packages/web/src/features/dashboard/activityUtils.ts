import type { IssueSummaryRecord, JobSummaryRecord, PatchId } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";

/**
 * Activity state for a subtask in the watchlist feed.
 * Ordered by visual priority (highest first).
 */
export type ActivityState =
  | "active"
  | "needs-review"
  | "blocked"
  | "failed"
  | "done"
  | "idle-open";

export interface ActivityItem {
  issueId: string;
  issue: IssueSummaryRecord;
  parentIssueId: string;
  parentDescription: string;
  state: ActivityState;
  /** Running/pending job for "active" items */
  activeJob?: JobSummaryRecord;
  /** Patch IDs for "needs-review" items */
  patchIds: PatchId[];
  /** Timestamp used for sorting within a group */
  sortTime: string;
}

export interface ActivitySummary {
  activeCount: number;
  attentionCount: number;
  doneCount: number;
  totalCount: number;
}

/**
 * Classify a subtask into an activity state based on its status, jobs, and dependencies.
 */
export function classifyActivity(
  node: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): ActivityState {
  const status = node.issue.issue.status;
  const jobs = jobsByIssue.get(node.id) ?? [];
  const hasRunningJob = jobs.some(
    (j) => j.task.status === "running" || j.task.status === "pending",
  );

  // Active: has a running/pending job
  if (hasRunningJob) return "active";

  // Failed: status is failed/rejected/dropped
  if (status === "failed" || status === "rejected" || status === "dropped") {
    return "failed";
  }

  // Done: status is closed
  if (status === "closed") return "done";

  // Blocked: has blocked-on dependency on non-closed issue
  if (node.blocked) return "blocked";

  // Needs review: in-progress, has patches, no running job
  if (status === "in-progress" && node.issue.issue.patches.length > 0) {
    return "needs-review";
  }

  // Idle-open: open or in-progress with no active work
  return "idle-open";
}

/**
 * Flatten an issue tree into a list of activity items for the feed.
 * Only collects leaf/subtask nodes (children of root), not root issues themselves.
 */
export function collectActivityItems(
  roots: IssueTreeNode[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): ActivityItem[] {
  const items: ActivityItem[] = [];

  function walkChildren(
    node: IssueTreeNode,
    parentId: string,
    parentDesc: string,
  ) {
    if (node.hardBlocked) return;

    const state = classifyActivity(node, jobsByIssue);
    const jobs = jobsByIssue.get(node.id) ?? [];
    const activeJob = jobs.find(
      (j) => j.task.status === "running" || j.task.status === "pending",
    );

    items.push({
      issueId: node.id,
      issue: node.issue,
      parentIssueId: parentId,
      parentDescription: parentDesc,
      state,
      activeJob,
      patchIds: node.issue.issue.patches,
      sortTime: activeJob?.task.start_time ?? node.issue.timestamp,
    });

    for (const child of node.children) {
      walkChildren(child, parentId, parentDesc);
    }
  }

  for (const root of roots) {
    const rootDesc = root.issue.issue.description.split("\n")[0].trim();
    for (const child of root.children) {
      walkChildren(child, root.id, rootDesc);
    }
  }

  return items;
}

/** Sort priority for activity states (lower = shown first) */
const STATE_PRIORITY: Record<ActivityState, number> = {
  active: 0,
  "needs-review": 1,
  blocked: 2,
  failed: 3,
  "idle-open": 4,
  done: 5,
};

/**
 * Sort activity items by priority: active first, then attention, then done.
 * Within each group, sort by sortTime (most recent first).
 */
export function sortActivityItems(items: ActivityItem[]): ActivityItem[] {
  return [...items].sort((a, b) => {
    const priorityDiff = STATE_PRIORITY[a.state] - STATE_PRIORITY[b.state];
    if (priorityDiff !== 0) return priorityDiff;
    return new Date(b.sortTime).getTime() - new Date(a.sortTime).getTime();
  });
}

/**
 * Compute global summary counts from activity items.
 */
export function computeSummary(items: ActivityItem[]): ActivitySummary {
  let activeCount = 0;
  let attentionCount = 0;
  let doneCount = 0;

  for (const item of items) {
    switch (item.state) {
      case "active":
        activeCount++;
        break;
      case "needs-review":
      case "blocked":
      case "failed":
        attentionCount++;
        break;
      case "done":
        doneCount++;
        break;
    }
  }

  return { activeCount, attentionCount, doneCount, totalCount: items.length };
}

/**
 * Compute per-root-issue progress breakdown for the sidebar.
 */
export interface IssueProgress {
  rootId: string;
  rootIssue: IssueSummaryRecord;
  done: number;
  active: number;
  needsAttention: number;
  open: number;
  failed: number;
  total: number;
}

export function computeIssueProgress(
  roots: IssueTreeNode[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): IssueProgress[] {
  return roots.map((root) => {
    const progress: IssueProgress = {
      rootId: root.id,
      rootIssue: root.issue,
      done: 0,
      active: 0,
      needsAttention: 0,
      open: 0,
      failed: 0,
      total: 0,
    };

    function walk(node: IssueTreeNode) {
      for (const child of node.children) {
        if (child.hardBlocked) continue;
        progress.total++;
        const state = classifyActivity(child, jobsByIssue);
        switch (state) {
          case "done":
            progress.done++;
            break;
          case "active":
            progress.active++;
            break;
          case "needs-review":
          case "blocked":
            progress.needsAttention++;
            break;
          case "failed":
            progress.failed++;
            break;
          case "idle-open":
            progress.open++;
            break;
        }
        walk(child);
      }
    }

    walk(root);
    return progress;
  });
}

/**
 * Label for an activity state, displayed as a badge in the feed.
 */
export function stateLabel(state: ActivityState): string {
  switch (state) {
    case "active":
      return "ACTIVE";
    case "needs-review":
      return "NEEDS REVIEW";
    case "blocked":
      return "BLOCKED";
    case "failed":
      return "FAILED";
    case "done":
      return "DONE";
    case "idle-open":
      return "OPEN";
  }
}
