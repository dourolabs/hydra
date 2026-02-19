import { Avatar, Badge, type BadgeStatus } from "@metis/ui";
import type { Issue } from "../../api/issues";
import styles from "./IssueRow.module.css";

interface IssueRowProps {
  issue: Issue;
  dimmed?: boolean;
}

/** First line of the description, truncated. */
function descriptionSnippet(desc: string, max = 80): string {
  const line = desc.split("\n")[0].trim();
  if (line.length <= max) return line;
  return line.slice(0, max) + "\u2026";
}

const validStatuses: Set<string> = new Set([
  "open",
  "in-progress",
  "closed",
  "failed",
  "dropped",
  "blocked",
  "rejected",
]);

function toBadgeStatus(status: string): BadgeStatus {
  if (validStatuses.has(status)) return status as BadgeStatus;
  return "open";
}

export function IssueRow({ issue, dimmed }: IssueRowProps) {
  return (
    <span className={`${styles.row}${dimmed ? ` ${styles.dimmed}` : ""}`}>
      <Badge status={toBadgeStatus(issue.status)} />
      <span className={styles.id}>{issue.issue_id}</span>
      {issue.assignee && <Avatar name={issue.assignee} size="sm" />}
      <span className={styles.desc}>{descriptionSnippet(issue.description)}</span>
    </span>
  );
}
