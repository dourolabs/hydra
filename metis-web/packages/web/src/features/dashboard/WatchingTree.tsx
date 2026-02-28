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

const FAILED_STATUSES: Set<string> = new Set(["failed", "dropped", "rejected"]);

interface SubtreeSummary {
  open: number;
  inProgress: number;
  closed: number;
  failed: number;
}

function summarizeSubtree(node: IssueTreeNode): SubtreeSummary {
  const summary: SubtreeSummary = { open: 0, inProgress: 0, closed: 0, failed: 0 };

  function walk(n: IssueTreeNode) {
    for (const child of n.children) {
      if (child.hardBlocked) continue;
      const status = child.issue.issue.status;
      if (status === "in-progress") {
        summary.inProgress++;
      } else if (status === "closed") {
        summary.closed++;
      } else if (FAILED_STATUSES.has(status)) {
        summary.failed++;
      } else {
        summary.open++;
      }
      walk(child);
    }
  }

  walk(node);
  return summary;
}

/** SVG completion ring showing closed (green), failed (red), and remaining (gray) arcs. */
function CompletionRing({ summary }: { summary: SubtreeSummary }) {
  const total = summary.open + summary.inProgress + summary.closed + summary.failed;
  if (total === 0) return null;

  const terminal = summary.closed + summary.failed;
  const size = 18;
  const strokeWidth = 3;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;

  const closedFrac = summary.closed / total;
  const failedFrac = summary.failed / total;

  // Arcs drawn clockwise from 12 o'clock: green (closed), then red (failed), rest is gray track
  const closedLen = closedFrac * circumference;
  const failedLen = failedFrac * circumference;

  return (
    <span className={styles.completionRing}>
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        className={styles.ringSvg}
      >
        {/* Gray track (full circle background) */}
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--color-border)"
          strokeWidth={strokeWidth}
        />
        {/* Failed arc (red) — drawn after closed so it starts where closed ends */}
        {failedLen > 0 && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke="var(--color-status-failed)"
            strokeWidth={strokeWidth}
            strokeDasharray={`${failedLen} ${circumference - failedLen}`}
            strokeDashoffset={-closedLen}
            strokeLinecap="round"
            transform={`rotate(-90 ${size / 2} ${size / 2})`}
          />
        )}
        {/* Closed arc (green) — starts at 12 o'clock */}
        {closedLen > 0 && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke="var(--color-status-closed)"
            strokeWidth={strokeWidth}
            strokeDasharray={`${closedLen} ${circumference - closedLen}`}
            strokeLinecap="round"
            transform={`rotate(-90 ${size / 2} ${size / 2})`}
          />
        )}
      </svg>
      <span className={styles.fractionLabel}>
        {terminal}/{total}
      </span>
    </span>
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

  const totalChildren = summary.open + summary.inProgress + summary.closed + summary.failed;

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
        {totalChildren > 0 && <CompletionRing summary={summary} />}
        <IssueRow
          record={root.issue}
          blocked={root.blocked}
          jobs={jobsByIssue.get(root.id)}
          onJobClick={handleJobClick}
        />
      </button>
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
