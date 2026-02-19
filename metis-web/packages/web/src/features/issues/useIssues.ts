import { useQuery } from "@tanstack/react-query";
import type { IssueVersionRecord } from "@metis/api";
import { apiClient } from "../../api/client";

export function useIssues(q?: string) {
  return useQuery({
    queryKey: q ? ["issues", { q }] : ["issues"],
    queryFn: () => apiClient.listIssues(q ? { q } : undefined),
    select: (data) => data.issues,
  });
}

export interface IssueTreeNode {
  id: string;
  issue: IssueVersionRecord;
  children: IssueTreeNode[];
  defaultExpanded: boolean;
}

/**
 * Build a tree from a flat list of issues.
 * Parent-child relationships are derived from "child-of" dependencies:
 * if issue B has dependency { type: "child-of", issue_id: A }, then B is a child of A.
 */
export function buildIssueTree(issues: IssueVersionRecord[]): IssueTreeNode[] {
  const issueMap = new Map<string, IssueVersionRecord>();
  for (const record of issues) {
    issueMap.set(record.issue_id, record);
  }

  // Map issue_id -> children issue_ids
  const childrenMap = new Map<string, string[]>();
  const hasParent = new Set<string>();

  for (const record of issues) {
    for (const dep of record.issue.dependencies) {
      if (dep.type === "child-of") {
        hasParent.add(record.issue_id);
        const siblings = childrenMap.get(dep.issue_id) ?? [];
        siblings.push(record.issue_id);
        childrenMap.set(dep.issue_id, siblings);
      }
    }
  }

  function buildNode(record: IssueVersionRecord): IssueTreeNode {
    const childIds = childrenMap.get(record.issue_id) ?? [];
    const children = childIds
      .map((id) => issueMap.get(id))
      .filter((i): i is IssueVersionRecord => i !== undefined)
      .map(buildNode);

    return {
      id: record.issue_id,
      issue: record,
      children,
      defaultExpanded: true,
    };
  }

  // Root nodes are issues that have no parent (not in hasParent set)
  const roots = issues.filter((i) => !hasParent.has(i.issue_id));
  return roots.map(buildNode);
}
