import { Link, useParams } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { useIssue } from "../features/issues/useIssue";
import { IssueDetail } from "../features/issues/IssueDetail";
import { ApiError } from "../api/client";
import styles from "./IssueDetailPage.module.css";

export function IssueDetailPage() {
  const { issueId } = useParams<{ issueId: string }>();
  const { data: record, isLoading, error } = useIssue(issueId ?? "");

  return (
    <div className={styles.page}>
      <Link to="/issues" className={styles.back}>
        &larr; Back to issues
      </Link>

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <div className={styles.errorContainer}>
          {error instanceof ApiError && error.status === 404 ? (
            <p className={styles.error}>
              Issue <strong>{issueId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load issue: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && <IssueDetail record={record} />}
    </div>
  );
}
