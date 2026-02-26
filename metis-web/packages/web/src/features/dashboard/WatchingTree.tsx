import { useState, useMemo, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { Badge, JobStatusIndicator, TreeView } from "@metis/ui";
import type { JobSummary, TreeNode } from "@metis/ui";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { IssueRow } from "../issues/IssueRow";
import {
  buildIssueTree,
  type IssueTreeNode,
} from "../issues/useIssues";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { isNodeActive, pruneTree } from "./watchingUtils";
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
}

function summarizeSubtree(node: IssueTreeNode): SubtreeSummary {
  const summary: SubtreeSummary = { open: 0, inProgress: 0, closed: 0 };
  const TERMINAL_STATUSES = new Set(["closed", "failed", "dropped", "rejected"]);

  function walk(n: IssueTreeNode) {
    for (const child of n.children) {
      if (child.hardBlocked) continue;
      const status = child.issue.issue.status;
      if (status === "in-progress") {
        summary.inProgress++;
      } else if (TERMINAL_STATUSES.has(status)) {
        summary.closed++;
      } else {
        summary.open++;
      }
      walk(child);
    }
  }

  walk(node);
  return summary;
}

function collectActiveChildren(
  node: IssueTreeNode,
  jobsByIssue: Map<string, JobSummaryRecord[]>,
): IssueTreeNode[] {
  const result: IssueTreeNode[] = [];
  const seen = new Set<string>();

  function walk(n: IssueTreeNode) {
    for (const child of n.children) {
      if (child.hardBlocked) continue;
      if (!seen.has(child.id) && isNodeActive(child, jobsByIssue)) {
        seen.add(child.id);
        result.push(child);
      }
      walk(child);
    }
  }

  walk(node);
  return result.sort((a, b) => new Date(b.issue.creation_time).getTime() - new Date(a.issue.creation_time).getTime());
}

function toJobSummary(record: JobSummaryRecord): JobSummary {
  const status = record.task.status === "unknown" ? "created" : record.task.status;
  return {
    jobId: record.job_id,
    status,
    startTime: record.task.start_time,
    endTime: record.task.end_time,
  };
}

function formatSummary(summary: SubtreeSummary): string {
  const parts: string[] = [];
  if (summary.inProgress > 0) parts.push(`${summary.inProgress} in-progress`);
  if (summary.open > 0) parts.push(`${summary.open} open`);
  if (summary.closed > 0) parts.push(`${summary.closed} closed`);
  return parts.join(", ");
}

/**
 * Convert IssueTreeNodes to TreeView-compatible TreeNodes using IssueRow labels.
 */
function issueNodesToTreeNodes(
  nodes: IssueTreeNode[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
  onJobClick: (issueId: string, jobId: string) => void,
): TreeNode[] {
  return nodes
    .filter((node) => !node.hardBlocked)
    .map((node) => ({
      id: node.id,
      label: (
        <IssueRow
          record={node.issue}
          blocked={node.blocked}
          blockedBy={node.blockedBy}
          jobs={jobsByIssue.get(node.id)}
          onJobClick={onJobClick}
        />
      ),
      children:
        node.children.length > 0
          ? issueNodesToTreeNodes(node.children, jobsByIssue, onJobClick)
          : undefined,
      defaultExpanded: true,
    }));
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
      .filter((root) => !root.hardBlocked && root.issue.issue.creator === username && pruneTree(root, jobsByIssue) !== null)
      .sort((a, b) => new Date(b.issue.creation_time).getTime() - new Date(a.issue.creation_time).getTime());
  }, [issues, jobsByIssue, username]);

  if (watchingRoots.length === 0) {
    return <p className={styles.empty}>No issues being watched.</p>;
  }

  return (
    <ul className={styles.list}>
      {watchingRoots.map((root) => {
        const expanded = expandedRoots.has(root.id);
        const summary = summarizeSubtree(root);
        const summaryText = formatSummary(summary);
        const totalChildren = summary.open + summary.inProgress + summary.closed;

        // Build child TreeNodes based on expanded/collapsed state
        let childNodes: TreeNode[];
        if (expanded) {
          // Expanded: show full pruned tree
          const prunedNode = pruneTree(root, jobsByIssue);
          childNodes = prunedNode
            ? issueNodesToTreeNodes(prunedNode.children, jobsByIssue, handleJobClick)
            : [];
        } else {
          // Collapsed: show flat list of active descendants
          const activeChildren = collectActiveChildren(root, jobsByIssue);
          childNodes = activeChildren.map((child) => ({
            id: child.id,
            label: (
              <IssueRow
                record={child.issue}
                blocked={child.blocked}
                blockedBy={child.blockedBy}
                jobs={jobsByIssue.get(child.id)}
                onJobClick={handleJobClick}
              />
            ),
          }));
        }

        // Root node styling
        const rootClassNames = [styles.node];
        if (root.id === selectedId) rootClassNames.push(styles.active);
        if (root.blocked) rootClassNames.push(styles.blocked);
        if (root.issue.issue.assignee === username) rootClassNames.push(styles.assignedToMe);

        const jobs = jobsByIssue.get(root.id);
        const jobSummaries = jobs?.map(toJobSummary);

        return (
          <li key={root.id} className={styles.rootItem}>
            <button
              className={rootClassNames.join(" ")}
              onClick={() => onSelect(root.id)}
              type="button"
            >
              <span className={styles.topRow}>
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
                <Badge status={issueToBadgeStatus(root.issue.issue.status)} />
                {jobSummaries && jobSummaries.length > 0 && (
                  <span
                    className={styles.jobIndicator}
                    onClick={(e) => e.stopPropagation()}
                    role="presentation"
                  >
                    <JobStatusIndicator jobs={jobSummaries} onJobClick={(jobId) => handleJobClick(root.id, jobId)} />
                  </span>
                )}
                <span className={styles.id}>{root.id}</span>
              </span>
              <span className={styles.desc}>
                {descriptionSnippet(root.issue.issue.description, 50)}
              </span>
              {root.blocked && root.blockedBy.length > 0 && (
                <span className={styles.blockedBy}>blocked by {root.blockedBy.join(", ")}</span>
              )}
            </button>
            {summaryText && (
              <div className={styles.summary}>{summaryText}</div>
            )}
            {childNodes.length > 0 && (
              <div className={expanded ? styles.children : styles.inProgressSection}>
                <TreeView
                  nodes={childNodes}
                  onNodeClick={onSelect}
                  collapsedIds={subtreeCollapsedIds}
                  onToggle={handleSubtreeToggle}
                />
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}
