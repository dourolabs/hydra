import type { JobVersionRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";

const TERMINAL_STATUSES = new Set(["closed", "failed", "dropped", "rejected"]);

/**
 * A node is "active" if it is open/in-progress or has a running/pending job.
 */
export function isNodeActive(
  node: IssueTreeNode,
  jobsByIssue: Map<string, JobVersionRecord[]>,
): boolean {
  const status = node.issue.issue.status;
  if (status === "open" || status === "in-progress") return true;

  const jobs = jobsByIssue.get(node.id);
  if (jobs?.some((j) => j.task.status === "running" || j.task.status === "pending")) {
    return true;
  }

  return false;
}

/**
 * Recursively prune terminal branches from a tree.
 * A node is kept if it is active or has at least one active descendant.
 * Returns null if the entire subtree is terminal.
 */
export function pruneTree(
  node: IssueTreeNode,
  jobsByIssue: Map<string, JobVersionRecord[]>,
): IssueTreeNode | null {
  const prunedChildren = node.children
    .map((child) => pruneTree(child, jobsByIssue))
    .filter((child): child is IssueTreeNode => child !== null);

  if (isNodeActive(node, jobsByIssue) || prunedChildren.length > 0) {
    return { ...node, children: prunedChildren };
  }

  return null;
}

/**
 * Check whether any node in a tree is active.
 */
export function treeHasActiveNode(
  node: IssueTreeNode,
  jobsByIssue: Map<string, JobVersionRecord[]>,
): boolean {
  if (isNodeActive(node, jobsByIssue)) return true;
  return node.children.some((child) => treeHasActiveNode(child, jobsByIssue));
}
