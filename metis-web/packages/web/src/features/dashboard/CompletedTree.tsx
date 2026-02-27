import { useState, useMemo, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { IssueRow } from "../issues/IssueRow";
import {
  buildIssueTree,
  type IssueTreeNode,
} from "../issues/useIssues";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import styles from "./WatchingTree.module.css";

interface CompletedTreeProps {
  issues: IssueSummaryRecord[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  username: string;
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
      <IssueRow
        record={node.issue}
        blocked={node.blocked}
        blockedBy={node.blockedBy}
        jobs={jobsByIssue.get(node.id)}
        onJobClick={onJobClick}
      />
    </button>
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
  const toggle = useCallback(() => setExpanded((prev) => !prev), []);

  const visibleChildren = node.children.filter((c) => !c.hardBlocked);
  const hasChildren = visibleChildren.length > 0;

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
        hasChildren={hasChildren}
        username={username}
      />
      {expanded && hasChildren && (
        <ChildNodes
          nodes={node.children}
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

export function CompletedTree({
  issues,
  jobsByIssue,
  selectedId,
  onSelect,
  username,
}: CompletedTreeProps) {
  const navigate = useNavigate();

  const handleJobClick = useCallback(
    (issueId: string, jobId: string) => {
      navigate(`/issues/${issueId}/jobs/${jobId}/logs`);
    },
    [navigate],
  );

  const completedRoots = useMemo(() => {
    const tree = buildIssueTree(issues);
    return tree
      .filter(
        (root) =>
          !root.hardBlocked &&
          root.issue.issue.creator === username &&
          TERMINAL_STATUSES.has(root.issue.issue.status),
      )
      .sort(
        (a, b) =>
          new Date(b.issue.creation_time).getTime() -
          new Date(a.issue.creation_time).getTime(),
      );
  }, [issues, username]);

  if (completedRoots.length === 0) {
    return <p className={styles.empty}>No completed issues.</p>;
  }

  return (
    <ul className={styles.list}>
      {completedRoots.map((node) => (
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
