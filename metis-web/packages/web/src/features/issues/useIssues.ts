import { useQuery } from "@tanstack/react-query";
import type { IssueSummaryRecord } from "@metis/api";
import { apiClient } from "../../api/client";
import { computeBlockedStatus } from "./blockedStatus";
import { topologicalSort } from "./topologicalSort";

export function useIssues() {
  return useQuery({
    queryKey: ["issues"],
    queryFn: () => apiClient.listIssues(),
    select: (data) => data.issues,
  });
}

export interface IssueTreeNode {
  id: string;
  issue: IssueSummaryRecord;
  children: IssueTreeNode[];
  defaultExpanded: boolean;
  blocked: boolean;
  blockedBy: string[];
  hardBlocked: boolean;
  hardBlockedBy: string[];
}

/**
 * Build a tree from a flat list of issues.
 * Parent-child relationships are derived from "child-of" dependencies:
 * if issue B has dependency { type: "child-of", issue_id: A }, then B is a child of A.
 */
export function buildIssueTree(issues: IssueSummaryRecord[]): IssueTreeNode[] {
  const issueMap = new Map<string, IssueSummaryRecord>();
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

  function buildNode(record: IssueSummaryRecord): IssueTreeNode {
    const childIds = childrenMap.get(record.issue_id) ?? [];
    const childRecords = childIds
      .map((id) => issueMap.get(id))
      .filter((i): i is IssueSummaryRecord => i !== undefined);
    const children = topologicalSort(childRecords).map(buildNode);

    const status = computeBlockedStatus(record, issueMap);

    return {
      id: record.issue_id,
      issue: record,
      children,
      defaultExpanded: true,
      ...status,
    };
  }

  // Root nodes are issues that have no parent (not in hasParent set)
  const roots = issues.filter((i) => !hasParent.has(i.issue_id));
  return topologicalSort(roots).map(buildNode);
}
