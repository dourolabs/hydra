import { Link } from "react-router-dom";
import { Badge, Spinner, type BadgeStatus } from "@metis/ui";
import { useIssues } from "./useIssues";
import styles from "./IssueChildren.module.css";

interface IssueChildrenProps {
  issueId: string;
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

/** First line of the description, truncated. */
function descriptionSnippet(desc: string, max = 60): string {
  const line = desc.split("\n")[0].trim();
  if (line.length <= max) return line;
  return line.slice(0, max) + "\u2026";
}

export function IssueChildren({ issueId }: IssueChildrenProps) {
  const { data: allIssues, isLoading } = useIssues();

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  // Find children: issues that have a "child-of" dependency on this issueId
  const children = allIssues
    ? allIssues.filter((record) =>
        record.issue.dependencies.some(
          (dep) => dep.type === "child-of" && dep.issue_id === issueId,
        ),
      )
    : [];

  if (children.length === 0) {
    return <p className={styles.empty}>No child issues.</p>;
  }

  return (
    <ul className={styles.list}>
      {children.map((record) => (
        <li key={record.issue_id} className={styles.item}>
          <Badge status={toBadgeStatus(record.issue.status)} />
          <Link to={`/issues/${record.issue_id}`} className={styles.link}>
            <span className={styles.id}>{record.issue_id}</span>
            <span className={styles.desc}>
              {descriptionSnippet(record.issue.description)}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
