import { Link } from "react-router-dom";
import { Badge, Spinner, type BadgeStatus } from "@metis/ui";
import { useJobsByIssue } from "./useJobsByIssue";
import styles from "./JobList.module.css";

interface JobListProps {
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
  if (s) return s;
  if (validStatuses.has(status)) return status as BadgeStatus;
  return "open";
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
function getRuntime(startTime: string | null | undefined, endTime: string | null | undefined): string {
  if (!startTime) return "\u2014";
  const start = new Date(startTime).getTime();
  const end = endTime ? new Date(endTime).getTime() : Date.now();
  return formatDuration(end - start);
}

export function JobList({ issueId }: JobListProps) {
  const { data: jobs, isLoading, error } = useJobsByIssue(issueId);

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  if (error) {
    return (
      <p className={styles.error}>
        Failed to load jobs: {(error as Error).message}
      </p>
    );
  }

  if (!jobs || jobs.length === 0) {
    return <p className={styles.empty}>No jobs.</p>;
  }

  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.th}>Status</th>
          <th className={styles.th}>Job ID</th>
          <th className={styles.th}>Created</th>
          <th className={styles.th}>Runtime</th>
          <th className={styles.th}>Logs</th>
        </tr>
      </thead>
      <tbody>
        {jobs.map((record) => (
          <tr key={record.job_id} className={styles.row}>
            <td className={styles.td}>
              <Badge status={toBadgeStatus(record.task.status)} />
            </td>
            <td className={styles.td}>
              <span className={styles.jobId}>{record.job_id}</span>
            </td>
            <td className={styles.td}>
              <span className={styles.time}>
                {record.task.creation_time
                  ? new Date(record.task.creation_time).toLocaleString()
                  : "\u2014"}
              </span>
            </td>
            <td className={styles.td}>
              <span className={styles.time}>
                {getRuntime(record.task.start_time, record.task.end_time)}
              </span>
            </td>
            <td className={styles.td}>
              <Link
                to={`/issues/${issueId}/jobs/${record.job_id}/logs`}
                className={styles.logLink}
              >
                View Logs
              </Link>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
