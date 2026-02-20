import type { IssueTreeNode } from "../issues/useIssues";

export function containsAssignedOpen(node: IssueTreeNode, username: string): boolean {
  if (node.issue.issue.assignee === username && node.issue.issue.status === "open") {
    return true;
  }
  return node.children.some((child) => containsAssignedOpen(child, username));
}
