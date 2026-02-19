import { Link, useParams } from "react-router-dom";
import { Badge, Spinner, type BadgeStatus } from "@metis/ui";
import { useJob } from "../features/jobs/useJob";
import { JobLogViewer } from "../features/jobs/JobLogViewer";
import { ApiError } from "../api/client";
import styles from "./JobLogPage.module.css";

/** Map job statuses to BadgeStatus values. */
function toBadgeStatus(status: string): BadgeStatus {
  const mapped: Record<string, BadgeStatus> = {
    created: "open",
    pending: "open",
    running: "in-progress",
    complete: "closed",
    failed: "failed",
  };
  const s = mapped[status];
  return s ?? "open";
}

/** Format a duration in milliseconds to a human-readable string. */
function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) return `${minutes}m ${remainingSeconds}s`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}h ${remainingMinutes}m`;
}

/** Compute runtime from start_time to end_time (or now). */
function getRuntime(
  startTime: string | null | undefined,
  endTime: string | null | undefined,
): string {
  if (!startTime) return "\u2014";
  const start = new Date(startTime).getTime();
  const end = endTime ? new Date(endTime).getTime() : Date.now();
  return formatDuration(end - start);
}

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
              <Badge status={toBadgeStatus(record.task.status)} />
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
