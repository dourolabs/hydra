import { useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Avatar, Badge } from "@hydra/ui";
import type { SessionSummaryRecord, Status as SessionStatus } from "@hydra/api";
import { useAllSessions } from "../useAllSessions";
import { sortSessions } from "../sortSessions";
import { normalizeSessionStatus } from "../../../utils/statusMapping";
import { getRuntime } from "../../../utils/time";
import { descriptionSnippet } from "../../../utils/text";
import styles from "./SessionsView.module.css";

interface StatusFilter {
  key: "all" | SessionStatus;
  label: string;
}

const STATUS_FILTERS: StatusFilter[] = [
  { key: "all", label: "All" },
  { key: "running", label: "Running" },
  { key: "pending", label: "Pending" },
  { key: "complete", label: "Complete" },
  { key: "failed", label: "Failed" },
];

function relativeTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "—";
  const sec = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (sec < 60) return "now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day}d`;
  const mo = Math.floor(day / 30);
  return `${mo}mo`;
}

export function SessionsView() {
  const navigate = useNavigate();
  const [selectedStatus, setSelectedStatus] = useState<SessionStatus | null>(null);
  const { data, isLoading, error } = useAllSessions();

  const filteredSorted = useMemo<SessionSummaryRecord[]>(() => {
    const sorted = data ? sortSessions(data) : [];
    if (!selectedStatus) return sorted;
    return sorted.filter((r) => r.session.status === selectedStatus);
  }, [data, selectedStatus]);

  const handleRowClick = (id: string) => {
    navigate(`/sessions/${id}`);
  };

  const activeKey: StatusFilter["key"] = selectedStatus ?? "all";
  const totalLabel =
    filteredSorted.length === 1 ? "1 SESSION" : `${filteredSorted.length} SESSIONS`;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>WORK · {totalLabel}</span>
          <h1 className={styles.pageTitle}>Sessions</h1>
        </div>
        <span className={styles.headSpacer} />
      </div>

      <div className={styles.toolbar}>
        {STATUS_FILTERS.map((f) => (
          <button
            key={f.key}
            type="button"
            className={`${styles.chipFilter}${activeKey === f.key ? ` ${styles.chipFilterActive}` : ""}`}
            onClick={() => setSelectedStatus(f.key === "all" ? null : f.key)}
            data-testid={`sessions-filter-${f.key}`}
          >
            <span>{f.label}</span>
          </button>
        ))}
        <span className={styles.toolbarSpacer} />
      </div>

      <div className={styles.body}>
        {isLoading && filteredSorted.length === 0 && (
          <div className={styles.empty}>Loading sessions…</div>
        )}

        {error && (
          <div className={styles.empty}>Failed to load sessions: {error.message}</div>
        )}

        {!isLoading && !error && filteredSorted.length === 0 && (
          <div className={styles.empty}>No sessions match the current filters.</div>
        )}

        {filteredSorted.length > 0 && (
          <div className={styles.tableWrap}>
            <table className={styles.table} data-testid="sessions-list">
              <thead>
                <tr>
                  <th className={styles.colAgent}>Agent</th>
                  <th className={styles.colLinked}>Linked</th>
                  <th className={styles.colStatus}>Status</th>
                  <th className={styles.colStarted}>Started</th>
                  <th className={styles.colDuration}>Duration</th>
                </tr>
              </thead>
              <tbody>
                {filteredSorted.map((rec) => {
                  const s = rec.session;
                  const startedTs = s.start_time ?? s.creation_time ?? rec.timestamp;
                  const promptText = descriptionSnippet(s.prompt);
                  return (
                    <tr
                      key={rec.session_id}
                      onClick={() => handleRowClick(rec.session_id)}
                      data-testid={`sessions-list-row-${rec.session_id}`}
                    >
                      <td className={styles.colAgent}>
                        <span className={styles.agent}>
                          <Avatar name={s.creator} kind="agent" size="md" />
                          <span className={styles.agentName}>{s.creator}</span>
                        </span>
                      </td>
                      <td className={styles.colLinked}>
                        <div className={styles.linkedCell}>
                          <span className={styles.linkedText}>{promptText}</span>
                          {s.spawned_from && (
                            <Link
                              to={`/issues/${s.spawned_from}`}
                              className={styles.linkedIssueLink}
                              onClick={(e) => e.stopPropagation()}
                              title={s.spawned_from}
                            >
                              {s.spawned_from}
                            </Link>
                          )}
                        </div>
                      </td>
                      <td className={styles.colStatus}>
                        <Badge status={normalizeSessionStatus(s.status)} />
                      </td>
                      <td className={styles.colStarted}>{relativeTime(startedTs)}</td>
                      <td className={styles.colDuration}>
                        {getRuntime(s.start_time, s.end_time)}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
