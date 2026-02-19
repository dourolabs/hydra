import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { TreeView } from "@metis/ui";
import type { TreeNode } from "@metis/ui";
import { IssueRow } from "./IssueRow";
import { type IssueTreeNode, buildIssueTree } from "./useIssues";
import type { Issue } from "../../api/issues";

interface IssueTreeProps {
  issues: Issue[];
  /** When provided, only show branches containing these issue IDs. Non-matching ancestors are dimmed. */
  matchingIds?: Set<string>;
  className?: string;
}

/**
 * Check if a tree node or any of its descendants match the filter.
 * Returns true if the node itself or any child has an ID in matchingIds.
 */
function hasMatchingDescendant(node: IssueTreeNode, matchingIds: Set<string>): boolean {
  if (matchingIds.has(node.id)) return true;
  return node.children.some((child) => hasMatchingDescendant(child, matchingIds));
}

/**
 * Convert IssueTreeNodes into TreeNodes with IssueRow labels.
 * When matchingIds is provided, prune branches without matches and dim non-matching ancestors.
 */
function toTreeNodes(nodes: IssueTreeNode[], matchingIds?: Set<string>): TreeNode[] {
  const result: TreeNode[] = [];

  for (const node of nodes) {
    if (matchingIds && !hasMatchingDescendant(node, matchingIds)) {
      continue;
    }

    const dimmed = matchingIds ? !matchingIds.has(node.id) : false;

    result.push({
      id: node.id,
      label: <IssueRow issue={node.issue} dimmed={dimmed} />,
      children:
        node.children.length > 0
          ? toTreeNodes(node.children, matchingIds)
          : undefined,
      defaultExpanded: node.defaultExpanded,
    });
  }

  return result;
}

export function IssueTree({ issues, matchingIds, className }: IssueTreeProps) {
  const navigate = useNavigate();

  const tree = useMemo(() => {
    const issueNodes = buildIssueTree(issues);
    return toTreeNodes(issueNodes, matchingIds);
  }, [issues, matchingIds]);

  const handleNodeClick = (id: string) => {
    navigate(`/issues/${id}`);
  };

  return <TreeView nodes={tree} onNodeClick={handleNodeClick} className={className} />;
}
