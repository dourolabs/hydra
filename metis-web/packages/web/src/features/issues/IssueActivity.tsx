import { useQuery } from "@tanstack/react-query";
import { Badge } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { apiClient } from "../../api/client";
import { ActivityTimeline } from "../activity/ActivityTimeline";
import type { Change } from "../activity/types";
import styles from "../activity/ActivityTimeline.module.css";

interface IssueActivityProps {
  issueId: string;
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
    changes.push({ field: "progress" });
  }
  if (prevIssue.description !== currIssue.description) {
    changes.push({ field: "description" });
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

function IssueChangeEntry({ change }: { change: Change }) {
  if (change.field === "status" && change.before && change.after) {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Status</span>
        <span className={styles.statusTransition}>
          <Badge status={issueToBadgeStatus(change.before)} />
          <span className={styles.arrow}>{"\u2192"}</span>
          <Badge status={issueToBadgeStatus(change.after)} />
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
      </div>
    );
  }

  if (change.field === "description") {
    return (
      <div className={styles.change}>
        <span className={styles.changeLabel}>Description</span>
        updated
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
      renderChange={(change, i) => (
        <IssueChangeEntry key={i} change={change} />
      )}
    />
  );
}
