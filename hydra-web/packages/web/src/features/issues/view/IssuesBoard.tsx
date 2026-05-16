import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, Badge, TypeChip } from "@hydra/ui";
import type { IssueStatus, IssueSummaryRecord } from "@hydra/api";
import { normalizeIssueStatus } from "../../../utils/statusMapping";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import styles from "./IssuesBoard.module.css";

interface IssuesBoardProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  filterRootId: string | null;
}

interface BoardColumn {
  status: IssueStatus;
  label: string;
}

const COLUMNS: BoardColumn[] = [
  { status: "open", label: "Open" },
  { status: "in-progress", label: "In progress" },
  { status: "failed", label: "Failed" },
  { status: "closed", label: "Closed" },
  { status: "dropped", label: "Dropped" },
];

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

export function IssuesBoard({ issues, childStatusMap, filterRootId }: IssuesBoardProps) {
  const navigate = useNavigate();

  const grouped = useMemo(() => {
    const map = new Map<IssueStatus, IssueSummaryRecord[]>();
    for (const col of COLUMNS) map.set(col.status, []);
    for (const rec of issues) {
      const s = rec.issue.status as IssueStatus;
      if (!map.has(s)) continue;
      map.get(s)!.push(rec);
    }
    return map;
  }, [issues]);

  const handleCardClick = (id: string) => {
    const params = new URLSearchParams({
      from: "dashboard",
      filter: filterRootId ?? "everything",
    });
    navigate(`/issues/${id}?${params.toString()}`);
  };

  return (
    <div className={styles.kanban}>
      {COLUMNS.map((col) => {
        const colIssues = grouped.get(col.status) ?? [];
        return (
          <div key={col.status} className={styles.col}>
            <div className={styles.colHead}>
              <Badge status={normalizeIssueStatus(col.status)} />
              <span className={styles.colCount}>{colIssues.length}</span>
            </div>
            <div className={styles.colBody}>
              {colIssues.length === 0 && (
                <div className={styles.colEmpty}>No issues</div>
              )}
              {colIssues.map((rec) => {
                const issue = rec.issue;
                const id = rec.issue_id;
                const children = childStatusMap.get(id);
                const pct = progressFraction(children);

                return (
                  <div
                    key={id}
                    className={styles.card}
                    onClick={() => handleCardClick(id)}
                  >
                    {issue.type && issue.type !== "unknown" && (
                      <div className={styles.cardHead}>
                        <TypeChip type={issue.type} />
                      </div>
                    )}
                    <div className={styles.cardTitle}>{issue.title || "(untitled)"}</div>
                    <div className={styles.cardFoot}>
                      {issue.assignee && <Avatar name={issue.assignee} kind="human" size="md" />}
                      <span>{relativeTime(rec.timestamp)}</span>
                      <span className={styles.cardFootSpacer} />
                      {children && children.length > 0 && (
                        <div className={styles.progress} title={`${pct}%`}>
                          <span style={{ width: `${pct}%` }} />
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        );
      })}
    </div>
  );
}
