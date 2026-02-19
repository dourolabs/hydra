import { useMemo } from "react";
import { TreeView } from "@metis/ui";
import type { TreeNode } from "@metis/ui";
import { IssueRow } from "./IssueRow";
import { type IssueTreeNode, buildIssueTree } from "./useIssues";
import type { Issue } from "../../api/issues";

interface IssueTreeProps {
  issues: Issue[];
  className?: string;
}

/** Convert IssueTreeNodes into TreeNodes with IssueRow labels. */
function toTreeNodes(nodes: IssueTreeNode[]): TreeNode[] {
  return nodes.map((node) => ({
    id: node.id,
    label: <IssueRow issue={node.issue} />,
    children: node.children.length > 0 ? toTreeNodes(node.children) : undefined,
    defaultExpanded: node.defaultExpanded,
  }));
}

export function IssueTree({ issues, className }: IssueTreeProps) {
  const tree = useMemo(() => {
    const issueNodes = buildIssueTree(issues);
    return toTreeNodes(issueNodes);
  }, [issues]);

  return <TreeView nodes={tree} className={className} />;
}
