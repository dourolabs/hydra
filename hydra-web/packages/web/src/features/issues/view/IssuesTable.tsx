import { useNavigate } from "react-router-dom";
import { Avatar, Badge, TypeChip } from "@hydra/ui";
import type { IssueSummaryRecord } from "@hydra/api";
import { normalizeIssueStatus } from "../../../utils/statusMapping";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import styles from "./IssuesTable.module.css";

interface IssuesTableProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
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
  const done = children.filter(
    (c) => c.status === "closed" || c.status === "issue-closed",
  ).length;
  return Math.round((done / total) * 100);
}

export function IssuesTable({ issues, childStatusMap, filterRootId }: IssuesTableProps) {
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
            <th className={styles.colUpdated}>Updated</th>
            <th className={styles.colProgress}>Progress</th>
          </tr>
        </thead>
        <tbody>
          {issues.map((rec) => {
            const issue = rec.issue;
            const id = rec.issue_id;
            const status = normalizeIssueStatus(issue.status);
            const children = childStatusMap.get(id);
            const pct = progressFraction(children);

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
                <td className={styles.colUpdated}>{relativeTime(rec.timestamp)}</td>
                <td className={styles.colProgress}>
                  {children && children.length > 0 ? (
                    <div className={styles.progress} title={`${pct}%`}>
                      <span style={{ width: `${pct}%` }} />
                    </div>
                  ) : (
                    <span className={styles.dash}>—</span>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
