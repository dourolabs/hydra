import { Link } from "react-router-dom";
import { Badge, Spinner } from "@metis/ui";
import { normalizeSessionStatus } from "../../utils/statusMapping";
import { getRuntime } from "../../utils/time";
import { useSessionsByIssue } from "./useSessionsByIssue";
import styles from "./SessionList.module.css";

interface SessionListProps {
  issueId: string;
}

export function SessionList({ issueId }: SessionListProps) {
  const { data: sessions, isLoading, error } = useSessionsByIssue(issueId);

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  if (error) {
    return (
      <p className={styles.error}>
        Failed to load sessions: {(error as Error).message}
      </p>
    );
  }

  if (!sessions || sessions.length === 0) {
    return <p className={styles.empty}>No sessions.</p>;
  }

  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.th}>Status</th>
          <th className={styles.th}>Session ID</th>
          <th className={styles.th}>Created</th>
          <th className={styles.th}>Runtime</th>
          <th className={styles.th}>Logs</th>
        </tr>
      </thead>
      <tbody>
        {sessions.map((record) => (
          <tr key={record.job_id} className={styles.row}>
            <td className={styles.td}>
              <Badge status={normalizeSessionStatus(record.task.status)} />
            </td>
            <td className={styles.td}>
              <Link
                to={`/issues/${issueId}/sessions/${record.job_id}/logs`}
                className={styles.sessionId}
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
                to={`/issues/${issueId}/sessions/${record.job_id}/logs`}
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
