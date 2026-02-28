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
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { treeHasActiveNode } from "./watchingUtils";
import styles from "./WatchingTree.module.css";

interface WatchingTreeProps {
  issues: IssueSummaryRecord[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  username: string;
}

const STATUS_ORDER: Record<string, number> = {
  "in-progress": 0,
  open: 1,
  closed: 2,
  failed: 3,
  dropped: 4,
  rejected: 5,
};

const STATUS_DOT_CSS_VAR: Record<string, string> = {
  open: "var(--color-status-open)",
  "in-progress": "var(--color-status-in-progress)",
  closed: "var(--color-status-closed)",
  failed: "var(--color-status-failed)",
  dropped: "var(--color-status-dropped)",
  rejected: "var(--color-status-rejected)",
};

interface ChildStatus {
  id: string;
  status: string;
}

function collectDirectChildStatuses(node: IssueTreeNode): ChildStatus[] {
  return node.children
    .filter((child) => !child.hardBlocked)
    .map((child) => ({ id: child.id, status: child.issue.issue.status }))
    .sort((a, b) => (STATUS_ORDER[a.status] ?? 6) - (STATUS_ORDER[b.status] ?? 6));
}

function StatusDots({ children: childStatuses }: { children: ChildStatus[] }) {
  if (childStatuses.length === 0) return null;

  const done = childStatuses.filter((c) => TERMINAL_STATUSES.has(c.status)).length;

  return (
    <div className={styles.statusDots}>
      <div className={styles.dotsRow}>
        {childStatuses.map((child) => (
          <span
            key={child.id}
            className={styles.dot}
            style={{ backgroundColor: STATUS_DOT_CSS_VAR[child.status] ?? "var(--color-text-tertiary)" }}
            title={`${child.id}: ${child.status}`}
          />
        ))}
      </div>
      <span className={styles.doneLabel}>
        {done}/{childStatuses.length} done
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
  const childStatuses = useMemo(() => collectDirectChildStatuses(root), [root]);
  const totalChildren = childStatuses.length;

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
    // Collapsed: hide all children; the dots provide status at a glance
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
      <StatusDots>{childStatuses}</StatusDots>
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
