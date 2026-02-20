import { useMemo, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { TreeView } from "@metis/ui";
import type { TreeNode } from "@metis/ui";
import type { IssueVersionRecord, JobVersionRecord } from "@metis/api";
import { IssueRow } from "./IssueRow";
import { type IssueTreeNode, buildIssueTree } from "./useIssues";

interface IssueTreeProps {
  issues: IssueVersionRecord[];
  /** When provided, only show branches containing these issue IDs. Non-matching ancestors are dimmed. */
  matchingIds?: Set<string>;
  /** Jobs grouped by issue ID, used to render job status indicators. */
  jobsByIssue?: Map<string, JobVersionRecord[]>;
  /** Controlled collapse state: set of collapsed node IDs. */
  collapsedIds?: Set<string>;
  /** Called when a node's expand/collapse chevron is clicked. */
  onToggle?: (id: string) => void;
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
function toTreeNodes(
  nodes: IssueTreeNode[],
  matchingIds: Set<string> | undefined,
  jobsByIssue: Map<string, JobVersionRecord[]> | undefined,
  onJobClick: (issueId: string, jobId: string) => void,
): TreeNode[] {
  const result: TreeNode[] = [];

  for (const node of nodes) {
    if (matchingIds && !hasMatchingDescendant(node, matchingIds)) {
      continue;
    }

    const dimmed = matchingIds ? !matchingIds.has(node.id) : false;
    const jobs = jobsByIssue?.get(node.id);

    result.push({
      id: node.id,
      label: <IssueRow record={node.issue} dimmed={dimmed} jobs={jobs} onJobClick={onJobClick} />,
      children:
        node.children.length > 0
          ? toTreeNodes(node.children, matchingIds, jobsByIssue, onJobClick)
          : undefined,
      defaultExpanded: node.defaultExpanded,
    });
  }

  return result;
}

export function IssueTree({ issues, matchingIds, jobsByIssue, collapsedIds, onToggle, className }: IssueTreeProps) {
  const navigate = useNavigate();

  const handleJobClick = useCallback(
    (issueId: string, jobId: string) => {
      navigate(`/issues/${issueId}/jobs/${jobId}/logs`);
    },
    [navigate],
  );

  const tree = useMemo(() => {
    const issueNodes = buildIssueTree(issues);
    return toTreeNodes(issueNodes, matchingIds, jobsByIssue, handleJobClick);
  }, [issues, matchingIds, jobsByIssue, handleJobClick]);

  const handleNodeClick = (id: string) => {
    navigate(`/issues/${id}`);
  };

  return <TreeView nodes={tree} onNodeClick={handleNodeClick} collapsedIds={collapsedIds} onToggle={onToggle} className={className} />;
}
