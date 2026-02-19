import { useQuery } from "@tanstack/react-query";
import { fetchIssues, toIssue, type Issue } from "../../api/issues";

export function useIssues() {
  return useQuery({
    queryKey: ["issues"],
    queryFn: async () => {
      const data = await fetchIssues();
      return data.issues.map(toIssue);
    },
  });
}

export interface IssueTreeNode {
  id: string;
  issue: Issue;
  children: IssueTreeNode[];
  defaultExpanded: boolean;
}

/**
 * Build a tree from a flat list of issues.
 * Parent-child relationships are derived from "child-of" dependencies:
 * if issue B has dependency { type: "child-of", issue_id: A }, then B is a child of A.
 */
export function buildIssueTree(issues: Issue[]): IssueTreeNode[] {
  const issueMap = new Map<string, Issue>();
  for (const issue of issues) {
    issueMap.set(issue.issue_id, issue);
  }

  // Map issue_id -> children issue_ids
  const childrenMap = new Map<string, string[]>();
  const hasParent = new Set<string>();

  for (const issue of issues) {
    for (const dep of issue.dependencies) {
      if (dep.type === "child-of") {
        hasParent.add(issue.issue_id);
        const siblings = childrenMap.get(dep.issue_id) ?? [];
        siblings.push(issue.issue_id);
        childrenMap.set(dep.issue_id, siblings);
      }
    }
  }

  function buildNode(issue: Issue): IssueTreeNode {
    const childIds = childrenMap.get(issue.issue_id) ?? [];
    const children = childIds
      .map((id) => issueMap.get(id))
      .filter((i): i is Issue => i !== undefined)
      .map(buildNode);

    return {
      id: issue.issue_id,
      issue,
      children,
      defaultExpanded: true,
    };
  }

  // Root nodes are issues that have no parent (not in hasParent set)
  const roots = issues.filter((i) => !hasParent.has(i.issue_id));
  return roots.map(buildNode);
}
