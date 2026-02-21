import { Link } from "react-router-dom";
import { Badge, Spinner } from "@metis/ui";
import { jobToBadgeStatus } from "../../utils/statusMapping";
import { getRuntime } from "../../utils/time";
import { useJobsByIssue } from "./useJobsByIssue";
import styles from "./JobList.module.css";

interface JobListProps {
  issueId: string;
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
              <Badge status={jobToBadgeStatus(record.task.status)} />
            </td>
            <td className={styles.td}>
              <Link
                to={`/issues/${issueId}/jobs/${record.job_id}/logs`}
                className={styles.jobId}
              >
                {record.job_id}
              </Link>
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
