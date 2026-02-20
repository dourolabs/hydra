import { useState, useMemo, useCallback } from "react";
import { Badge, JobStatusIndicator } from "@metis/ui";
import type { JobSummary } from "@metis/ui";
import type { IssueVersionRecord, JobVersionRecord } from "@metis/api";
import {
  buildIssueTree,
  type IssueTreeNode,
} from "../issues/useIssues";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import styles from "./WatchingTree.module.css";

interface WatchingTreeProps {
  issues: IssueVersionRecord[];
  jobsByIssue: Map<string, JobVersionRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
}

interface SubtreeSummary {
  open: number;
  inProgress: number;
  closed: number;
}

const TERMINAL_STATUSES = new Set(["closed", "failed", "dropped", "rejected"]);

function summarizeSubtree(node: IssueTreeNode): SubtreeSummary {
  const summary: SubtreeSummary = { open: 0, inProgress: 0, closed: 0 };

  function walk(n: IssueTreeNode) {
    for (const child of n.children) {
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

function collectInProgressChildren(node: IssueTreeNode): IssueTreeNode[] {
  const result: IssueTreeNode[] = [];

  function walk(n: IssueTreeNode) {
    for (const child of n.children) {
      if (child.issue.issue.status === "in-progress") {
        result.push(child);
      }
      walk(child);
    }
  }

  walk(node);
  return result;
}

function toJobSummary(record: JobVersionRecord): JobSummary {
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
  depth,
  jobsByIssue,
  selectedId,
  onSelect,
  expanded,
  onToggle,
  hasChildren,
}: {
  node: IssueTreeNode;
  depth: number;
  jobsByIssue: Map<string, JobVersionRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  expanded: boolean;
  onToggle: () => void;
  hasChildren: boolean;
}) {
  const active = node.id === selectedId;
  const jobs = jobsByIssue.get(node.id);
  const jobSummaries = jobs?.map(toJobSummary);

  return (
    <button
      className={`${styles.node}${active ? ` ${styles.active}` : ""}`}
      style={{ paddingLeft: `${depth * 16 + 12}px` }}
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
      <Badge status={issueToBadgeStatus(node.issue.issue.status)} />
      {jobSummaries && jobSummaries.length > 0 && (
        <span
          className={styles.jobIndicator}
          onClick={(e) => e.stopPropagation()}
          role="presentation"
        >
          <JobStatusIndicator jobs={jobSummaries} />
        </span>
      )}
      <span className={styles.id}>{node.id}</span>
      <span className={styles.desc}>
        {descriptionSnippet(node.issue.issue.description, 50)}
      </span>
    </button>
  );
}

function RootTreeNode({
  node,
  jobsByIssue,
  selectedId,
  onSelect,
}: {
  node: IssueTreeNode;
  jobsByIssue: Map<string, JobVersionRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);

  const summary = useMemo(() => summarizeSubtree(node), [node]);
  const inProgressChildren = useMemo(
    () => collectInProgressChildren(node),
    [node],
  );
  const summaryText = formatSummary(summary);
  const totalChildren = summary.open + summary.inProgress + summary.closed;

  const toggle = useCallback(() => setExpanded((prev) => !prev), []);

  return (
    <li className={styles.rootItem}>
      <TreeNodeRow
        node={node}
        depth={0}
        jobsByIssue={jobsByIssue}
        selectedId={selectedId}
        onSelect={onSelect}
        expanded={expanded}
        onToggle={toggle}
        hasChildren={totalChildren > 0}
      />
      {summaryText && (
        <div className={styles.summary}>{summaryText}</div>
      )}
      {!expanded && inProgressChildren.length > 0 && (
        <div className={styles.inProgressSection}>
          {inProgressChildren.map((child) => (
            <TreeNodeRow
              key={child.id}
              node={child}
              depth={1}
              jobsByIssue={jobsByIssue}
              selectedId={selectedId}
              onSelect={onSelect}
              expanded={false}
              onToggle={() => {}}
              hasChildren={false}
            />
          ))}
        </div>
      )}
      {expanded && (
        <ChildNodes
          nodes={node.children}
          depth={1}
          jobsByIssue={jobsByIssue}
          selectedId={selectedId}
          onSelect={onSelect}
        />
      )}
    </li>
  );
}

function ChildNodes({
  nodes,
  depth,
  jobsByIssue,
  selectedId,
  onSelect,
}: {
  nodes: IssueTreeNode[];
  depth: number;
  jobsByIssue: Map<string, JobVersionRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
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

  return (
    <div className={styles.children}>
      {nodes.map((child) => {
        const isExpanded = expandedSet.has(child.id);
        const hasGrandchildren = child.children.length > 0;
        return (
          <div key={child.id}>
            <TreeNodeRow
              node={child}
              depth={depth}
              jobsByIssue={jobsByIssue}
              selectedId={selectedId}
              onSelect={onSelect}
              expanded={isExpanded}
              onToggle={() => toggle(child.id)}
              hasChildren={hasGrandchildren}
            />
            {isExpanded && hasGrandchildren && (
              <ChildNodes
                nodes={child.children}
                depth={depth + 1}
                jobsByIssue={jobsByIssue}
                selectedId={selectedId}
                onSelect={onSelect}
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
}: WatchingTreeProps) {
  const watchingRoots = useMemo(() => {
    const tree = buildIssueTree(issues);
    return tree.filter((node) => {
      const status = node.issue.issue.status;
      return status === "open" || status === "in-progress";
    });
  }, [issues]);

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
        />
      ))}
    </ul>
  );
}

