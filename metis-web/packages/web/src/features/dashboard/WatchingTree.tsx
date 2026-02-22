import { useState, useMemo, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { Badge, JobStatusIndicator } from "@metis/ui";
import type { JobSummary } from "@metis/ui";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
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
  return result;
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

function TreeNodeRow({
  node,
  jobsByIssue,
  selectedId,
  onSelect,
  onJobClick,
  expanded,
  onToggle,
  hasChildren,
  username,
}: {
  node: IssueTreeNode;
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  onJobClick: (issueId: string, jobId: string) => void;
  expanded: boolean;
  onToggle: () => void;
  hasChildren: boolean;
  username: string;
}) {
  const active = node.id === selectedId;
  const jobs = jobsByIssue.get(node.id);
  const jobSummaries = jobs?.map(toJobSummary);

  const handleJobClick = useCallback(
    (jobId: string) => {
      onJobClick(node.id, jobId);
    },
    [onJobClick, node.id],
  );

  const classNames = [styles.node];
  if (active) classNames.push(styles.active);
  if (node.blocked) classNames.push(styles.blocked);
  if (node.issue.issue.assignee === username) classNames.push(styles.assignedToMe);

  return (
    <button
      className={classNames.join(" ")}
      onClick={() => onSelect(node.id)}
      type="button"
    >
      <span className={styles.topRow}>
        <span
          className={styles.chevron}
          onClick={(e) => {
            e.stopPropagation();
            onToggle();
          }}
          role="button"
          tabIndex={-1}
        >
          {hasChildren ? (expanded ? "\u25BE" : "\u25B8") : " "}
        </span>
        <Badge status={issueToBadgeStatus(node.issue.issue.status)} />
        {jobSummaries && jobSummaries.length > 0 && (
          <span
            className={styles.jobIndicator}
            onClick={(e) => e.stopPropagation()}
            role="presentation"
          >
            <JobStatusIndicator jobs={jobSummaries} onJobClick={handleJobClick} />
          </span>
        )}
        <span className={styles.id}>{node.id}</span>
      </span>
      <span className={styles.desc}>
        {descriptionSnippet(node.issue.issue.description, 50)}
      </span>
      {node.blocked && node.blockedBy.length > 0 && (
        <span className={styles.blockedBy}>blocked by {node.blockedBy.join(", ")}</span>
      )}
    </button>
  );
}

function RootTreeNode({
  node,
  jobsByIssue,
  selectedId,
  onSelect,
  onJobClick,
  username,
}: {
  node: IssueTreeNode;
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  onJobClick: (issueId: string, jobId: string) => void;
  username: string;
}) {
  const [expanded, setExpanded] = useState(false);

  const summary = useMemo(() => summarizeSubtree(node), [node]);
  const activeChildren = useMemo(
    () => collectActiveChildren(node, jobsByIssue),
    [node, jobsByIssue],
  );
  // Pruned tree for expanded rendering (excludes terminal-only branches)
  const prunedNode = useMemo(() => pruneTree(node, jobsByIssue), [node, jobsByIssue]);
  const summaryText = formatSummary(summary);
  const totalChildren = summary.open + summary.inProgress + summary.closed;

  const toggle = useCallback(() => setExpanded((prev) => !prev), []);

  return (
    <li className={styles.rootItem}>
      <TreeNodeRow
        node={node}
        jobsByIssue={jobsByIssue}
        selectedId={selectedId}
        onSelect={onSelect}
        onJobClick={onJobClick}
        expanded={expanded}
        onToggle={toggle}
        hasChildren={totalChildren > 0}
        username={username}
      />
      {summaryText && (
        <div className={styles.summary}>{summaryText}</div>
      )}
      {!expanded && activeChildren.length > 0 && (
        <div className={styles.inProgressSection}>
          {activeChildren.map((child) => (
            <TreeNodeRow
              key={child.id}
              node={child}
              jobsByIssue={jobsByIssue}
              selectedId={selectedId}
              onSelect={onSelect}
              onJobClick={onJobClick}
              expanded={false}
              onToggle={() => {}}
              hasChildren={false}
              username={username}
            />
          ))}
        </div>
      )}
      {expanded && prunedNode && (
        <ChildNodes
          nodes={prunedNode.children}
          jobsByIssue={jobsByIssue}
          selectedId={selectedId}
          onSelect={onSelect}
          onJobClick={onJobClick}
          username={username}
        />
      )}
    </li>
  );
}

function ChildNodes({
  nodes,
  jobsByIssue,
  selectedId,
  onSelect,
  onJobClick,
  username,
}: {
  nodes: IssueTreeNode[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  onJobClick: (issueId: string, jobId: string) => void;
  username: string;
}) {
  const [expandedSet, setExpandedSet] = useState<Set<string>>(() => new Set());

  const toggle = useCallback((id: string) => {
    setExpandedSet((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const visibleNodes = nodes.filter((child) => !child.hardBlocked);

  return (
    <div className={styles.children}>
      {visibleNodes.map((child) => {
        const isExpanded = expandedSet.has(child.id);
        const hasGrandchildren = child.children.filter((c) => !c.hardBlocked).length > 0;
        return (
          <div key={child.id}>
            <TreeNodeRow
              node={child}
              jobsByIssue={jobsByIssue}
              selectedId={selectedId}
              onSelect={onSelect}
              onJobClick={onJobClick}
              expanded={isExpanded}
              onToggle={() => toggle(child.id)}
              hasChildren={hasGrandchildren}
              username={username}
            />
            {isExpanded && hasGrandchildren && (
              <ChildNodes
                nodes={child.children}
                jobsByIssue={jobsByIssue}
                selectedId={selectedId}
                onSelect={onSelect}
                onJobClick={onJobClick}
                username={username}
              />
            )}
          </div>
        );
      })}
    </div>
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

  const handleJobClick = useCallback(
    (issueId: string, jobId: string) => {
      navigate(`/issues/${issueId}/jobs/${jobId}/logs`);
    },
    [navigate],
  );

  const watchingRoots = useMemo(() => {
    const tree = buildIssueTree(issues);
    // Keep full (unpruned) roots so that summarizeSubtree sees all children.
    // Use pruneTree only to decide whether the root has any active nodes.
    // Hide hard-blocked root issues entirely.
    return tree.filter((root) => !root.hardBlocked && pruneTree(root, jobsByIssue) !== null);
  }, [issues, jobsByIssue]);

  if (watchingRoots.length === 0) {
    return <p className={styles.empty}>No issues being watched.</p>;
  }

  return (
    <ul className={styles.list}>
      {watchingRoots.map((node) => (
        <RootTreeNode
          key={node.id}
          node={node}
          jobsByIssue={jobsByIssue}
          selectedId={selectedId}
          onSelect={onSelect}
          onJobClick={handleJobClick}
          username={username}
        />
      ))}
    </ul>
  );
}
