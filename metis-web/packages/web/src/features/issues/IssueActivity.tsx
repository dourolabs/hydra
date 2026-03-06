import { useRef, useState, useCallback, useEffect, useLayoutEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { Badge } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { normalizeIssueStatus } from "../../utils/statusMapping";
import { apiClient } from "../../api/client";
import { ActivityTimeline } from "../activity/ActivityTimeline";
import type { Change } from "../activity/types";
import styles from "../activity/ActivityTimeline.module.css";

interface IssueActivityProps {
  issueId: string;
}

const STATUS_DOT_COLORS: Record<string, string> = {
  open: "var(--color-status-open)",
  "in-progress": "var(--color-status-in-progress)",
  closed: "var(--color-status-closed)",
  failed: "var(--color-status-failed)",
  dropped: "var(--color-status-dropped)",
  rejected: "var(--color-status-rejected)",
};

function getDotColor(
  changes: Change[],
  isCreation: boolean,
): string | undefined {
  if (isCreation) {
    return "var(--color-accent)";
  }

  const statusChange = changes.find((c) => c.field === "status");
  if (statusChange?.after) {
    return STATUS_DOT_COLORS[statusChange.after];
  }

  return "var(--color-text-tertiary)";
}

function diffIssueVersions(
  prev: IssueVersionRecord,
  curr: IssueVersionRecord,
): Change[] {
  const changes: Change[] = [];
  const prevIssue = prev.issue;
  const currIssue = curr.issue;

  if (prevIssue.status !== currIssue.status) {
    changes.push({
      field: "status",
      before: prevIssue.status,
      after: currIssue.status,
    });
  }
  if (prevIssue.assignee !== currIssue.assignee) {
    changes.push({
      field: "assignee",
      before: prevIssue.assignee ?? "unassigned",
      after: currIssue.assignee ?? "unassigned",
    });
  }
  if (prevIssue.progress !== currIssue.progress) {
    changes.push({ field: "progress", value: currIssue.progress });
  }
  if (prevIssue.description !== currIssue.description) {
    changes.push({ field: "description", value: currIssue.description });
  }
  if (prevIssue.type !== currIssue.type) {
    changes.push({
      field: "type",
      before: prevIssue.type,
      after: currIssue.type,
    });
  }

  const prevPatches = new Set(prevIssue.patches);
  const currPatches = new Set(currIssue.patches);
  for (const p of currPatches) {
    if (!prevPatches.has(p)) {
      changes.push({ field: "patch", after: p });
    }
  }

  const prevDeps = JSON.stringify(prevIssue.dependencies);
  const currDeps = JSON.stringify(currIssue.dependencies);
  if (prevDeps !== currDeps) {
    changes.push({ field: "dependencies" });
  }

  return changes;
}

function ProgressValue({ value }: { value: string }) {
  const contentRef = useRef<HTMLDivElement>(null);
  const [truncated, setTruncated] = useState(false);
  const [expanded, setExpanded] = useState(false);

  useLayoutEffect(() => {
    const el = contentRef.current;
    if (el) {
      setTruncated(el.scrollHeight > el.clientHeight);
    }
  }, [value]);

  useEffect(() => {
    const el = contentRef.current;
    if (!el) return;

    const observer = new ResizeObserver(() => {
      setTruncated(el.scrollHeight > el.clientHeight);
    });
    observer.observe(el);

    return () => observer.disconnect();
  }, [value]);

  const toggle = useCallback(() => setExpanded((v) => !v), []);

  return (
    <div>
      <div
        ref={contentRef}
        className={
          expanded
            ? styles.progressContentExpanded
            : styles.progressContentTruncated
        }
      >
        {value}
      </div>
      {truncated && (
        <button
          type="button"
          className={styles.collapsibleSummary}
          onClick={toggle}
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

function IssueChangeEntry({ change }: { change: Change }) {
  if (change.field === "status" && change.before && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Status</span>
        <span className={styles.statusTransition}>
          <Badge status={normalizeIssueStatus(change.before)} />
          <span className={styles.arrow}>{"\u2192"}</span>
          <Badge status={normalizeIssueStatus(change.after)} />
        </span>
      </div>
    );
  }

  if (change.field === "assignee") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Assignee</span>
        <span className={styles.statusTransition}>
          {change.before ?? "unassigned"}
          <span className={styles.arrow}>{"\u2192"}</span>
          {change.after ?? "unassigned"}
        </span>
      </div>
    );
  }

  if (change.field === "progress") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Progress</span>
        updated
        {change.value && <ProgressValue value={change.value} />}
      </div>
    );
  }

  if (change.field === "description") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Description</span>
        updated
        {change.value && (
          <details>
            <summary className={styles.collapsibleSummary}>
              Show changes
            </summary>
            <div className={styles.collapsibleContent}>{change.value}</div>
          </details>
        )}
      </div>
    );
  }

  if (change.field === "type" && change.before && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Type</span>
        <span className={styles.statusTransition}>
          {change.before}
          <span className={styles.arrow}>{"\u2192"}</span>
          {change.after}
        </span>
      </div>
    );
  }

  if (change.field === "patch" && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Patch</span>
        {change.after} linked
      </div>
    );
  }

  if (change.field === "dependencies") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Dependencies</span>
        updated
      </div>
    );
  }

  return null;
}

function CreationSubItems({ version }: { version: IssueVersionRecord }) {
  const issue = version.issue;
  return (
    <div className={styles.creationSubItems}>
      <span className={styles.creationSubItem}>
        <span className={styles.creationSubItemLabel}>Type:</span>
        {issue.type}
      </span>
      <span className={styles.creationSubItem}>
        <span className={styles.creationSubItemLabel}>Status:</span>
        {issue.status}
      </span>
      <span className={styles.creationSubItem}>
        <span className={styles.creationSubItemLabel}>Assignee:</span>
        {issue.assignee ?? "unassigned"}
      </span>
    </div>
  );
}

export function IssueActivity({ issueId }: IssueActivityProps) {
  const { data, isLoading } = useQuery({
    queryKey: ["issue", issueId, "versions"],
    queryFn: () => apiClient.listIssueVersions(issueId),
  });
  const versions = data?.versions ?? [];

  return (
    <ActivityTimeline
      versions={versions}
      isLoading={isLoading}
      diffFn={diffIssueVersions}
      creationLabel="Issue created"
      getDotColor={getDotColor}
      renderCreation={(version) => <CreationSubItems version={version} />}
      renderChange={(change, i) => (
        <IssueChangeEntry key={i} change={change} />
      )}
    />
  );
}
