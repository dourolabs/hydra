import type { JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";

export interface StatusPill {
  kind: "active" | "review" | "failed" | "queued" | "complete" | "on-track";
  count: number;
  label: string;
}

export interface CardMetrics {
  total: number;
  done: number;
  active: number;
  review: number;
  failed: number;
  open: number;
  pills: StatusPill[];
}

export interface GlobalMetrics {
  agentsRunning: number;
  needAttention: number;
  shipped: number;
  total: number;
}

function hasRunningJob(
  nodeId: string,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): boolean {
  const jobs = jobsByIssue.get(nodeId);
  return jobs?.some(
    (j) => j.task.status === "running" || j.task.status === "pending",
  ) ?? false;
}

function hasOpenPatch(node: IssueTreeNode): boolean {
  return node.issue.issue.patches.length > 0;
}

/**
 * Classify each direct and indirect subtask of a root issue into stages.
 * Walks the full subtree (skipping hard-blocked nodes).
 */
export function computeCardMetrics(
  root: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): CardMetrics {
  let done = 0;
  let active = 0;
  let review = 0;
  let failed = 0;
  let open = 0;

  function walk(node: IssueTreeNode) {
    for (const child of node.children) {
      if (child.hardBlocked) continue;

      const status = child.issue.issue.status;
      const running = hasRunningJob(child.id, jobsByIssue);

      if (status === "closed") {
        done++;
      } else if (status === "failed" || status === "rejected" || status === "dropped") {
        failed++;
      } else if (running) {
        active++;
      } else if (
        status === "in-progress" &&
        hasOpenPatch(child) &&
        !running
      ) {
        review++;
      } else {
        open++;
      }

      walk(child);
    }
  }

  walk(root);

  const total = done + active + review + failed + open;
  const pills: StatusPill[] = [];

  if (active > 0) {
    pills.push({ kind: "active", count: active, label: `${active} active` });
  }
  if (review > 0) {
    pills.push({ kind: "review", count: review, label: `${review} review` });
  }
  if (failed > 0) {
    pills.push({ kind: "failed", count: failed, label: `${failed} failed` });
  }

  if (pills.length === 0) {
    if (open > 0) {
      pills.push({ kind: "queued", count: open, label: `${open} queued` });
    } else if (total > 0 && done === total) {
      pills.push({ kind: "complete", count: done, label: "complete" });
    } else {
      pills.push({ kind: "on-track", count: 0, label: "on track" });
    }
  }

  return { total, done, active, review, failed, open, pills };
}

/**
 * Aggregate metrics across all watched root issues for the global status bar.
 */
export function computeGlobalMetrics(
  roots: IssueTreeNode[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): GlobalMetrics {
  let agentsRunning = 0;
  let needAttention = 0;
  let shipped = 0;
  let total = 0;

  for (const root of roots) {
    const metrics = computeCardMetrics(root, jobsByIssue);
    agentsRunning += metrics.active;
    needAttention += metrics.review + metrics.failed;
    shipped += metrics.done;
    total += metrics.total;
  }

  return { agentsRunning, needAttention, shipped, total };
}

/**
 * Flatten all subtasks from a tree node for the expanded detail view.
 * Returns direct + indirect children (skipping hard-blocked).
 */
export interface FlatSubtask {
  id: string;
  issue: IssueTreeNode;
  depth: number;
  hasRunningJob: boolean;
}

export function flattenSubtasks(
  root: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): FlatSubtask[] {
  const result: FlatSubtask[] = [];

  function walk(node: IssueTreeNode, depth: number) {
    for (const child of node.children) {
      if (child.hardBlocked) continue;
      result.push({
        id: child.id,
        issue: child,
        depth,
        hasRunningJob: hasRunningJob(child.id, jobsByIssue),
      });
      walk(child, depth + 1);
    }
  }

  walk(root, 0);
  return result;
}
