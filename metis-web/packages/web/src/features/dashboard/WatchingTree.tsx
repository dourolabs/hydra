import { useState, useMemo, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { TreeView } from "@metis/ui";
import type { TreeNode } from "@metis/ui";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { IssueRow } from "../issues/IssueRow";
import {
  buildIssueTree,
  type IssueTreeNode,
} from "../issues/useIssues";
import { treeHasActiveNode } from "./watchingUtils";
import styles from "./WatchingTree.module.css";

interface WatchingTreeProps {
  issues: IssueSummaryRecord[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  username: string;
}

interface SubtreeSummary {
  open: number;
  inProgress: number;
  closed: number;
  failed: number;
  dropped: number;
  rejected: number;
}

function summarizeSubtree(node: IssueTreeNode): SubtreeSummary {
  const summary: SubtreeSummary = {
    open: 0,
    inProgress: 0,
    closed: 0,
    failed: 0,
    dropped: 0,
    rejected: 0,
  };

  function walk(n: IssueTreeNode) {
    for (const child of n.children) {
      if (child.hardBlocked) continue;
      const status = child.issue.issue.status;
      switch (status) {
        case "in-progress":
          summary.inProgress++;
          break;
        case "closed":
          summary.closed++;
          break;
        case "failed":
          summary.failed++;
          break;
        case "dropped":
          summary.dropped++;
          break;
        case "rejected":
          summary.rejected++;
          break;
        default:
          summary.open++;
          break;
      }
      walk(child);
    }
  }

  walk(node);
  return summary;
}

function formatTooltip(summary: SubtreeSummary): string {
  const parts: string[] = [];
  if (summary.inProgress > 0) parts.push(`${summary.inProgress} in-progress`);
  if (summary.open > 0) parts.push(`${summary.open} open`);
  if (summary.closed > 0) parts.push(`${summary.closed} closed`);
  if (summary.failed > 0) parts.push(`${summary.failed} failed`);
  if (summary.dropped > 0) parts.push(`${summary.dropped} dropped`);
  if (summary.rejected > 0) parts.push(`${summary.rejected} rejected`);
  return parts.join(", ");
}

const SEGMENT_CONFIG: { key: keyof SubtreeSummary; className: string }[] = [
  { key: "inProgress", className: styles.segmentInProgress },
  { key: "open", className: styles.segmentOpen },
  { key: "closed", className: styles.segmentClosed },
  { key: "failed", className: styles.segmentFailed },
  { key: "dropped", className: styles.segmentDropped },
  { key: "rejected", className: styles.segmentRejected },
];

function SegmentedProgressBar({ summary }: { summary: SubtreeSummary }) {
  const total =
    summary.open +
    summary.inProgress +
    summary.closed +
    summary.failed +
    summary.dropped +
    summary.rejected;

  if (total === 0) return null;

  const done = summary.closed;
  const tooltip = formatTooltip(summary);

  return (
    <div className={styles.progressContainer} title={tooltip}>
      <div className={styles.progressBar}>
        {SEGMENT_CONFIG.map(
          ({ key, className }) =>
            summary[key] > 0 && (
              <div
                key={key}
                className={className}
                style={{ width: `${(summary[key] / total) * 100}%` }}
              />
            ),
        )}
      </div>
      <span className={styles.progressLabel}>
        {done} / {total} done
      </span>
    </div>
  );
}

/**
 * Convert IssueTreeNodes to TreeView-compatible TreeNodes using IssueRow labels.
 * All visible (non-hard-blocked) children are rendered inline regardless of status.
 */
function issueNodesToTreeNodes(
  nodes: IssueTreeNode[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
  onJobClick: (issueId: string, jobId: string) => void,
  username: string,
): TreeNode[] {
  return nodes
    .filter((n) => !n.hardBlocked)
    .map((node) => ({
      id: node.id,
      label: (
        <IssueRow
          record={node.issue}
          blocked={node.blocked}
          jobs={jobsByIssue.get(node.id)}
          onJobClick={onJobClick}
        />
      ),
      className: node.issue.issue.assignee === username ? styles.assignedToMe : undefined,
      children:
        node.children.length > 0
          ? issueNodesToTreeNodes(
              node.children,
              jobsByIssue,
              onJobClick,
              username,
            )
          : undefined,
    }));
}

interface RootItemProps {
  root: IssueTreeNode;
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  expanded: boolean;
  toggleRoot: (id: string) => void;
  subtreeCollapsedIds: Set<string>;
  handleSubtreeToggle: (id: string) => void;
  handleJobClick: (issueId: string, jobId: string) => void;
  username: string;
}

function RootItem({
  root,
  jobsByIssue,
  selectedId,
  onSelect,
  expanded,
  toggleRoot,
  subtreeCollapsedIds,
  handleSubtreeToggle,
  handleJobClick,
  username,
}: RootItemProps) {
  const summary = useMemo(() => summarizeSubtree(root), [root]);

  const totalChildren =
    summary.open +
    summary.inProgress +
    summary.closed +
    summary.failed +
    summary.dropped +
    summary.rejected;

  // Build child TreeNodes based on expanded/collapsed state
  let childNodes: TreeNode[];
  if (expanded) {
    // Expanded: show full tree with all children inline
    childNodes = issueNodesToTreeNodes(
      root.children,
      jobsByIssue,
      handleJobClick,
      username,
    );
  } else {
    // Collapsed: hide all children; the summary text provides status at a glance
    childNodes = [];
  }

  // Root node styling
  const rootClassNames = [styles.node];
  if (root.id === selectedId) rootClassNames.push(styles.active);
  if (root.blocked) rootClassNames.push(styles.blocked);
  if (root.issue.issue.assignee === username) rootClassNames.push(styles.assignedToMe);

  return (
    <li className={styles.rootItem}>
      <button
        className={rootClassNames.join(" ")}
        onClick={() => onSelect(root.id)}
        type="button"
      >
        <span
          className={styles.chevron}
          onClick={(e) => {
            e.stopPropagation();
            toggleRoot(root.id);
          }}
          role="button"
          tabIndex={-1}
        >
          {totalChildren > 0 ? (expanded ? "\u25BE" : "\u25B8") : " "}
        </span>
        <IssueRow
          record={root.issue}
          blocked={root.blocked}
          jobs={jobsByIssue.get(root.id)}
          onJobClick={handleJobClick}
        />
      </button>
      {totalChildren > 0 && <SegmentedProgressBar summary={summary} />}
      {childNodes.length > 0 && (
        <div className={styles.children}>
          <TreeView
            nodes={childNodes}
            onNodeClick={onSelect}
            collapsedIds={subtreeCollapsedIds}
            onToggle={handleSubtreeToggle}
            selectedId={selectedId ?? undefined}
          />
        </div>
      )}
    </li>
  );
}

export function WatchingTree({
  issues,
  jobsByIssue,
  selectedId,
  onSelect,
  username,
}: WatchingTreeProps) {
  const navigate = useNavigate();
  // Stable state for which root nodes are expanded (default: all collapsed)
  const [expandedRoots, setExpandedRoots] = useState<Set<string>>(new Set());
  // Stable collapse state for subtree nodes — survives SSE-driven re-renders
  const [subtreeCollapsedIds, setSubtreeCollapsedIds] = useState<Set<string>>(new Set());
  const handleJobClick = useCallback(
    (issueId: string, jobId: string) => {
      navigate(`/issues/${issueId}/jobs/${jobId}/logs`);
    },
    [navigate],
  );

  const handleSubtreeToggle = useCallback((id: string) => {
    setSubtreeCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const toggleRoot = useCallback((id: string) => {
    setExpandedRoots((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const watchingRoots = useMemo(() => {
    const tree = buildIssueTree(issues);
    return tree
      .filter((root) => !root.hardBlocked && root.issue.issue.creator === username && treeHasActiveNode(root, jobsByIssue))
      .sort((a, b) => new Date(b.issue.creation_time).getTime() - new Date(a.issue.creation_time).getTime());
  }, [issues, jobsByIssue, username]);

  if (watchingRoots.length === 0) {
    return <p className={styles.empty}>No issues being watched.</p>;
  }

  return (
    <ul className={styles.list}>
      {watchingRoots.map((root) => (
        <RootItem
          key={root.id}
          root={root}
          jobsByIssue={jobsByIssue}
          selectedId={selectedId}
          onSelect={onSelect}
          expanded={expandedRoots.has(root.id)}
          toggleRoot={toggleRoot}
          subtreeCollapsedIds={subtreeCollapsedIds}
          handleSubtreeToggle={handleSubtreeToggle}
          handleJobClick={handleJobClick}
          username={username}
        />
      ))}
    </ul>
  );
}
