import { useCallback } from "react";
import { Avatar, Badge, SessionStatusIndicator } from "@hydra/ui";
import type { IssueSummaryRecord, IssueType, SessionSummaryRecord } from "@hydra/api";
import { toSessionSummary } from "../../utils/sessionMapping";
import { normalizeIssueStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatRelativeTime } from "../../utils/time";
import styles from "./IssueRow.module.css";

const typeChipClass: Record<IssueType, string> = {
  task: styles.task,
  bug: styles.bug,
  feature: styles.feature,
  chore: styles.chore,
  "merge-request": styles.mergeRequest,
  "review-request": styles.reviewRequest,
  unknown: styles.unknown,
};

interface IssueRowProps {
  record: IssueSummaryRecord;
  dimmed?: boolean;
  blocked?: boolean;
  sessions?: SessionSummaryRecord[];
  onSessionClick?: (issueId: string, sessionId: string) => void;
  showId?: boolean;
  showTimestamp?: boolean;
}

export function IssueRow({
  record,
  dimmed,
  blocked,
  sessions,
  onSessionClick,
  showId,
  showTimestamp,
}: IssueRowProps) {
  const { issue } = record;

  const handleSessionClick = useCallback(
    (sessionId: string) => {
      onSessionClick?.(record.issue_id, sessionId);
    },
    [onSessionClick, record.issue_id],
  );

  const sessionSummaries = sessions?.map(toSessionSummary);

  const classNames = [styles.row];
  if (dimmed) classNames.push(styles.dimmed);
  if (blocked) classNames.push(styles.blocked);

  const chipClass = typeChipClass[issue.type];

  return (
    <span className={classNames.join(" ")}>
      <span className={styles.topRow}>
        <Badge status={normalizeIssueStatus(issue.status)} />
        <span className={`${styles.typeChip} ${chipClass}`}>{issue.type}</span>
        {blocked && <span className={styles.blockedLabel}>BLOCKED</span>}
        <span className={styles.desc}>
          {issue.title ? (
            <><span className={styles.title}>{issue.title}</span>{" "}{descriptionSnippet(issue.description)}</>
          ) : (
            descriptionSnippet(issue.description)
          )}
        </span>
        {sessionSummaries && sessionSummaries.length > 0 && (
          <span
            className={styles.sessionIndicator}
            onClick={(e) => e.stopPropagation()}
            role="presentation"
          >
            <SessionStatusIndicator sessions={sessionSummaries} onSessionClick={handleSessionClick} />
          </span>
        )}
        {issue.assignee && <Avatar name={issue.assignee} size="sm" />}
      </span>
      {(showId || showTimestamp) && (
        <span className={styles.bottomRow}>
          {showId && <span className={styles.issueId}>{record.issue_id}</span>}
          {showTimestamp && record.timestamp && (
            <span className={styles.timestamp}>{formatRelativeTime(record.timestamp)}</span>
          )}
        </span>
      )}
    </span>
  );
}
