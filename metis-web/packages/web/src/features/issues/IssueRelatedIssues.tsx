import { Link } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { IssueRow } from "./IssueRow";
import { useIssues } from "./useIssues";
import { topologicalSort } from "./topologicalSort";
import styles from "./IssueRelatedIssues.module.css";

interface IssueRelatedIssuesProps {
  issueId: string;
}

export function IssueRelatedIssues({ issueId }: IssueRelatedIssuesProps) {
  const { data: allIssues, isLoading } = useIssues();

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  // Find current issue to read its parent dependencies
  const currentIssue = allIssues?.find((r) => r.issue_id === issueId);
  const parentIds =
    currentIssue?.issue.dependencies
      .filter((dep) => dep.type === "child-of")
      .map((dep) => dep.issue_id) ?? [];
  const parents = allIssues
    ? allIssues.filter((r) => parentIds.includes(r.issue_id))
    : [];

  // Find children: issues that have a "child-of" dependency on this issueId
  const children = allIssues
    ? topologicalSort(
        allIssues.filter((record) =>
          record.issue.dependencies.some(
            (dep) => dep.type === "child-of" && dep.issue_id === issueId,
          ),
        ),
      )
    : [];

  if (parents.length === 0 && children.length === 0) {
    return (
      <div className={styles.empty}>
        <p className={styles.emptyText}>No related issues.</p>
      </div>
    );
  }

  return (
    <>
      {parents.length > 0 && (
        <>
          <div className={styles.sectionLabel}>Parents</div>
          <ul className={styles.list}>
            {parents.map((record) => (
              <li key={record.issue_id} className={styles.item}>
                <Link
                  to={`/issues/${record.issue_id}`}
                  className={styles.link}
                >
                  <IssueRow record={record} showId />
                </Link>
              </li>
            ))}
          </ul>
        </>
      )}
      {children.length > 0 && (
        <>
          <div className={styles.sectionLabel}>Children</div>
          <ul className={styles.list}>
            {children.map((record) => (
              <li key={record.issue_id} className={styles.item}>
                <Link
                  to={`/issues/${record.issue_id}`}
                  className={styles.link}
                >
                  <IssueRow record={record} showId />
                </Link>
              </li>
            ))}
          </ul>
        </>
      )}
    </>
  );
}
