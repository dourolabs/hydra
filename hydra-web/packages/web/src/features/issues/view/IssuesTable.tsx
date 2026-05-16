import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, Badge, TypeChip } from "@hydra/ui";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";
import { normalizeIssueStatus } from "../../../utils/statusMapping";
import { formatDuration } from "../../../utils/time";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import styles from "./IssuesTable.module.css";

interface IssuesTableProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  filterRootId: string | null;
}

function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "";
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

function progressFraction(children: ChildStatus[] | undefined): number {
  if (!children || children.length === 0) return 0;
  const total = children.length;
  const projected = children.filter(
    (c) => c.status === "closed" || c.status === "issue-closed" || c.status === "in-progress",
  ).length;
  return Math.round((projected / total) * 100);
}

function isActiveSession(s: SessionSummaryRecord): boolean {
  return s.session.status === "running" || s.session.status === "pending";
}

function pickActiveSession(
  sessions: SessionSummaryRecord[] | undefined,
): SessionSummaryRecord | undefined {
  if (!sessions || sessions.length === 0) return undefined;
  return sessions.find(isActiveSession);
}

function pickLatestCompletedSession(
  sessions: SessionSummaryRecord[] | undefined,
): SessionSummaryRecord | undefined {
  if (!sessions || sessions.length === 0) return undefined;
  let best: SessionSummaryRecord | undefined;
  let bestTs = -Infinity;
  for (const s of sessions) {
    if (isActiveSession(s)) continue;
    const ts = s.session.end_time
      ? new Date(s.session.end_time).getTime()
      : new Date(s.timestamp).getTime();
    if (Number.isFinite(ts) && ts > bestTs) {
      bestTs = ts;
      best = s;
    }
  }
  return best;
}

function useElapsed(startIso: string | null | undefined, active: boolean): string {
  const compute = () => {
    if (!startIso) return "0s";
    return formatDuration(Date.now() - new Date(startIso).getTime());
  };
  const [text, setText] = useState<string>(compute);

  useEffect(() => {
    if (!active || !startIso) {
      setText(compute());
      return;
    }
    setText(compute());
    const id = setInterval(() => setText(compute()), 1000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, startIso]);

  return text;
}

function RuntimeCell({ sessions }: { sessions: SessionSummaryRecord[] | undefined }) {
  const active = pickActiveSession(sessions);
  const startIso = active?.session.start_time ?? active?.session.creation_time ?? null;
  const elapsed = useElapsed(startIso, !!active);

  if (active) {
    return (
      <span className={styles.runtimeActive} data-testid="runtime-active">
        {elapsed}
      </span>
    );
  }

  const completed = pickLatestCompletedSession(sessions);
  if (completed && completed.session.start_time) {
    const start = new Date(completed.session.start_time).getTime();
    const end = completed.session.end_time ? new Date(completed.session.end_time).getTime() : start;
    return (
      <span className={styles.runtimeIdle} data-testid="runtime-idle">
        {formatDuration(end - start)}
      </span>
    );
  }

  return <span className={styles.dash}>—</span>;
}

export function IssuesTable({
  issues,
  childStatusMap,
  sessionsByIssue,
  filterRootId,
}: IssuesTableProps) {
  const navigate = useNavigate();

  const handleRowClick = (id: string) => {
    const params = new URLSearchParams({
      from: "dashboard",
      filter: filterRootId ?? "everything",
    });
    navigate(`/issues/${id}?${params.toString()}`);
  };

  return (
    <div className={styles.tableWrap}>
      <table className={styles.table}>
        <thead>
          <tr>
            <th className={styles.colTitle}>Title</th>
            <th className={styles.colStatus}>Status</th>
            <th className={styles.colType}>Type</th>
            <th className={styles.colAssignee}>Assignee</th>
            <th className={styles.colProgress}>Progress</th>
            <th className={styles.colRuntime}>Runtime</th>
            <th className={styles.colUpdated}>Updated</th>
          </tr>
        </thead>
        <tbody>
          {issues.map((rec) => {
            const issue = rec.issue;
            const id = rec.issue_id;
            const status = normalizeIssueStatus(issue.status);
            const children = childStatusMap.get(id);
            const pct = progressFraction(children);
            const hasActiveChild = !!children?.some((c) => c.hasActiveTask);
            const progressClass = hasActiveChild
              ? `${styles.progress} ${styles.progressActive}`
              : styles.progress;
            const fillClass = hasActiveChild
              ? `${styles.progressFill} ${styles.progressFillActive}`
              : styles.progressFill;

            return (
              <tr key={id} onClick={() => handleRowClick(id)}>
                <td className={styles.colTitle}>
                  <div className={styles.titleCell}>
                    <span className={styles.titleText}>{issue.title || "(untitled)"}</span>
                  </div>
                </td>
                <td className={styles.colStatus}>
                  <Badge status={status} />
                </td>
                <td className={styles.colType}>
                  {issue.type && issue.type !== "unknown" ? (
                    <TypeChip type={issue.type} />
                  ) : (
                    <span className={styles.dash}>—</span>
                  )}
                </td>
                <td className={styles.colAssignee}>
                  {issue.assignee ? (
                    <span className={styles.assignee}>
                      <Avatar name={issue.assignee} kind="human" size="md" />
                      <span className={styles.assigneeName}>{issue.assignee}</span>
                    </span>
                  ) : (
                    <span className={styles.dash}>—</span>
                  )}
                </td>
                <td className={styles.colProgress}>
                  {children && children.length > 0 ? (
                    <div className={progressClass} title={`${pct}%`}>
                      <span className={fillClass} style={{ width: `${pct}%` }} />
                    </div>
                  ) : (
                    <span className={styles.dash}>—</span>
                  )}
                </td>
                <td className={styles.colRuntime}>
                  <RuntimeCell sessions={sessionsByIssue.get(id)} />
                </td>
                <td className={styles.colUpdated}>{relativeTime(rec.timestamp)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
