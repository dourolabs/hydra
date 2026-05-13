import { useMemo } from "react";
import { Link } from "react-router-dom";
import { Badge } from "@hydra/ui";
import type { SessionSummaryRecord } from "@hydra/api";
import { useAllSessions } from "../features/sessions/useAllSessions";
import { sortSessionsActiveFirst } from "../features/sessions/sortSessions";
import { LoadingState } from "../components/LoadingState/LoadingState";
import { ErrorState } from "../components/ErrorState/ErrorState";
import { EmptyState } from "../components/EmptyState/EmptyState";
import { normalizeSessionStatus } from "../utils/statusMapping";
import { formatTimestamp } from "../utils/time";
import styles from "./SessionsListPage.module.css";

const PROMPT_PREVIEW_MAX = 80;

function promptPreview(prompt: string): string {
  const trimmed = prompt.trim();
  if (trimmed.length <= PROMPT_PREVIEW_MAX) return trimmed;
  return `${trimmed.slice(0, PROMPT_PREVIEW_MAX - 1)}…`;
}

function sessionDetailHref(record: SessionSummaryRecord): string | null {
  const issueId = record.session.spawned_from;
  if (!issueId) return null;
  return `/issues/${issueId}/sessions/${record.session_id}/logs`;
}

export function SessionsListPage() {
  const { data, isLoading, error, refetch } = useAllSessions();

  const sorted = useMemo(() => sortSessionsActiveFirst(data ?? []), [data]);

  return (
    <div className={styles.page}>
      <div className={styles.headerRow}>
        <h2 className={styles.title}>Sessions</h2>
      </div>

      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load sessions: ${(error as Error).message}`}
          onRetry={() => refetch()}
        />
      )}

      {!isLoading && !error && sorted.length === 0 && (
        <EmptyState message="No sessions yet." />
      )}

      {sorted.length > 0 && (
        <ul className={styles.list} data-testid="sessions-list">
          {sorted.map((record) => {
            const href = sessionDetailHref(record);
            const issueId = record.session.spawned_from;
            return (
              <li
                key={record.session_id}
                className={styles.row}
                data-testid={`session-row-${record.session_id}`}
              >
                <div className={styles.rowMain}>
                  {href ? (
                    <Link to={href} className={styles.sessionId}>
                      {record.session_id}
                    </Link>
                  ) : (
                    <span className={styles.sessionIdPlain}>
                      {record.session_id}
                    </span>
                  )}
                  <Badge
                    status={normalizeSessionStatus(record.session.status)}
                  />
                </div>
                <div className={styles.prompt}>
                  {promptPreview(record.session.prompt)}
                </div>
                <div className={styles.rowMeta}>
                  <span className={styles.metaItem}>{record.session.creator}</span>
                  {issueId && (
                    <Link
                      to={`/issues/${issueId}`}
                      className={styles.issueLink}
                    >
                      {issueId}
                    </Link>
                  )}
                  <span className={styles.metaItem}>
                    {record.session.creation_time
                      ? `created ${formatTimestamp(record.session.creation_time)}`
                      : "—"}
                  </span>
                  {record.session.end_time && (
                    <span className={styles.metaItem}>
                      ended {formatTimestamp(record.session.end_time)}
                    </span>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
