import { Link } from "react-router-dom";
import { Badge, Spinner } from "@metis/ui";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { useIssues } from "./useIssues";
import styles from "./IssueChildren.module.css";

interface IssueChildrenProps {
  issueId: string;
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
          <Badge status={issueToBadgeStatus(record.issue.status)} />
          <Link to={`/issues/${record.issue_id}`} className={styles.link}>
            <span className={styles.id}>{record.issue_id}</span>
            <span className={styles.desc}>
              {descriptionSnippet(record.issue.description, 60)}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
