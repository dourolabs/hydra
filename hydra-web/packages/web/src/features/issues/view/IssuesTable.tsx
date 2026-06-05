import { useNavigate } from "react-router-dom";
import { Avatar, TypeChip } from "@hydra/ui";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../../principal/formatPrincipal";
import { StatusChip } from "../../projects/StatusChip";
import { useMediaQuery } from "../../../hooks/useMediaQuery";
import { AgoTime, RunTime } from "../../../components/Runtime/Runtime";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import { useSessionDuration } from "../../dashboard/useSessionDuration";
import { IssueRailRow } from "../../related/RailRow";
import styles from "./IssuesTable.module.css";

const MOBILE_QUERY = "(max-width: 768px)";

interface IssuesTableProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  filterRootId: string | null;
}

function progressFraction(children: ChildStatus[] | undefined): number {
  if (!children || children.length === 0) return 0;
  const total = children.length;
  const projected = children.filter(
    (c) => c.status === "closed" || c.status === "issue-closed" || c.status === "in-progress",
  ).length;
  return Math.round((projected / total) * 100);
}

function RuntimeCell({ sessions }: { sessions: SessionSummaryRecord[] | undefined }) {
  const { durationText, status } = useSessionDuration(sessions);
  if (durationText === "—") return <span className={styles.dash}>—</span>;
  return <RunTime value={durationText} status={status} />;
}

export function IssuesTable({
  issues,
  childStatusMap,
  sessionsByIssue,
  filterRootId,
}: IssuesTableProps) {
  const navigate = useNavigate();
  const isMobile = useMediaQuery(MOBILE_QUERY);

  const linkSearch =
    "?" +
    new URLSearchParams({
      from: "dashboard",
      filter: filterRootId ?? "everything",
    }).toString();

  if (isMobile) {
    return (
      <div className={styles.mobileList}>
        {issues.map((rec) => (
          <IssueRailRow
            key={rec.issue_id}
            record={rec}
            sessions={sessionsByIssue.get(rec.issue_id)}
            childStatuses={childStatusMap.get(rec.issue_id)}
            linkSearch={linkSearch}
          />
        ))}
      </div>
    );
  }

  const handleRowClick = (id: string) => {
    navigate(`/issues/${id}${linkSearch}`);
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
              <tr
                key={id}
                data-testid={`issues-list-row-${id}`}
                onClick={() => handleRowClick(id)}
              >
                <td className={styles.colTitle}>
                  <div className={styles.titleCell}>
                    <span className={styles.titleText}>{issue.title || "(untitled)"}</span>
                  </div>
                </td>
                <td className={styles.colStatus}>
                  <StatusChip
                    definition={issue.resolved_status}
                    fallbackKey={issue.status}
                  />
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
                      <Avatar
                        name={principalDisplayName(issue.assignee)}
                        kind={principalAvatarKind(issue.assignee)}
                        size="md"
                      />
                      <span className={styles.assigneeName}>
                        {principalDisplayName(issue.assignee)}
                      </span>
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
                <td className={styles.colUpdated}>
                  <AgoTime iso={rec.timestamp} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
