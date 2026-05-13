import { useMemo } from "react";
import { Link } from "react-router-dom";
import { Badge } from "@hydra/ui";
import { useAllSessions } from "../features/sessions/useAllSessions";
import { sortSessions } from "../features/sessions/sortSessions";
import { LoadingState } from "../components/LoadingState/LoadingState";
import { ErrorState } from "../components/ErrorState/ErrorState";
import { EmptyState } from "../components/EmptyState/EmptyState";
import { normalizeSessionStatus } from "../utils/statusMapping";
import { formatTimestamp } from "../utils/time";
import { descriptionSnippet } from "../utils/text";
import styles from "./SessionsListPage.module.css";

export function SessionsListPage() {
  const { data, isLoading, error, refetch } = useAllSessions();

  const sorted = useMemo(() => (data ? sortSessions(data) : []), [data]);

  return (
    <div className={styles.page}>
      <div className={styles.headerRow}>
        <h2 className={styles.title}>Sessions</h2>
      </div>

      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load sessions: ${error.message}`}
          onRetry={() => refetch()}
        />
      )}

      {!isLoading && !error && sorted.length === 0 && (
        <EmptyState message="No sessions yet." />
      )}

      {sorted.length > 0 && (
        <ul className={styles.list} data-testid="sessions-list">
          {sorted.map((record) => {
            const spawnedFrom = record.session.spawned_from;
            const sessionLink = spawnedFrom
              ? `/issues/${spawnedFrom}/sessions/${record.session_id}/logs`
              : null;
            const time =
              record.session.end_time ??
              record.session.start_time ??
              record.session.creation_time ??
              record.timestamp;
            return (
              <li
                key={record.session_id}
                className={styles.row}
                data-testid={`sessions-list-row-${record.session_id}`}
              >
                <div className={styles.rowMain}>
                  {sessionLink ? (
                    <Link to={sessionLink} className={styles.sessionId}>
                      {record.session_id}
                    </Link>
                  ) : (
                    <span className={styles.sessionId}>
                      {record.session_id}
                    </span>
                  )}
                  <Badge
                    status={normalizeSessionStatus(record.session.status)}
                  />
                  <span className={styles.prompt}>
                    {descriptionSnippet(record.session.prompt)}
                  </span>
                </div>
                <div className={styles.rowMeta}>
                  <span className={styles.metaItem}>{record.session.creator}</span>
                  {spawnedFrom && (
                    <Link
                      to={`/issues/${spawnedFrom}`}
                      className={styles.issueLink}
                    >
                      {spawnedFrom}
                    </Link>
                  )}
                  <span className={styles.metaItem}>{formatTimestamp(time)}</span>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
