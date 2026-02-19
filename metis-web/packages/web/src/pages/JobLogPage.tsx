import { Link, useParams } from "react-router-dom";
import { Badge, Spinner } from "@metis/ui";
import { jobToBadgeStatus } from "../utils/statusMapping";
import { getRuntime } from "../utils/time";
import { useJob } from "../features/jobs/useJob";
import { JobLogViewer } from "../features/jobs/JobLogViewer";
import { ApiError } from "../api/client";
import styles from "./JobLogPage.module.css";

export function JobLogPage() {
  const { issueId, jobId } = useParams<{
    issueId: string;
    jobId: string;
  }>();
  const { data: record, isLoading, error } = useJob(jobId ?? "");

  return (
    <div className={styles.page}>
      <Link to={`/issues/${issueId}`} className={styles.back}>
        &larr; Back to issue
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
              Job <strong>{jobId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load job: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && (
        <>
          {/* Job metadata header */}
          <div className={styles.header}>
            <div className={styles.headerTop}>
              <span className={styles.jobId}>{record.job_id}</span>
              <Badge status={jobToBadgeStatus(record.task.status)} />
            </div>
            <div className={styles.meta}>
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Issue</span>
                <Link to={`/issues/${issueId}`} className={styles.metaLink}>
                  {issueId}
                </Link>
              </div>
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Runtime</span>
                <span className={styles.metaValue}>
                  {getRuntime(record.task.start_time, record.task.end_time)}
                </span>
              </div>
              {record.task.creation_time && (
                <div className={styles.metaItem}>
                  <span className={styles.metaLabel}>Created</span>
                  <span className={styles.metaValue}>
                    {new Date(record.task.creation_time).toLocaleString()}
                  </span>
                </div>
              )}
            </div>
          </div>

          {/* Log viewer */}
          <JobLogViewer jobId={record.job_id} status={record.task.status} />
        </>
      )}
    </div>
  );
}
