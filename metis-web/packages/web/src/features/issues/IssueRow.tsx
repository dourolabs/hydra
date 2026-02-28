import { useCallback } from "react";
import { Avatar, Badge, JobStatusIndicator } from "@metis/ui";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { toJobSummary } from "../../utils/jobMapping";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatRelativeTime } from "../../utils/time";
import styles from "./IssueRow.module.css";

const typeChipClass: Record<string, string> = {
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
  jobs?: JobSummaryRecord[];
  onJobClick?: (issueId: string, jobId: string) => void;
  showId?: boolean;
  showTimestamp?: boolean;
}

export function IssueRow({
  record,
  dimmed,
  blocked,
  jobs,
  onJobClick,
  showId,
  showTimestamp,
}: IssueRowProps) {
  const { issue } = record;

  const handleJobClick = useCallback(
    (jobId: string) => {
      onJobClick?.(record.issue_id, jobId);
    },
    [onJobClick, record.issue_id],
  );

  const jobSummaries = jobs?.map(toJobSummary);

  const classNames = [styles.row];
  if (dimmed) classNames.push(styles.dimmed);
  if (blocked) classNames.push(styles.blocked);

  const chipClass = typeChipClass[issue.type] ?? styles.unknown;

  return (
    <span className={classNames.join(" ")}>
      <span className={styles.topRow}>
        <Badge status={issueToBadgeStatus(issue.status)} />
        <span className={`${styles.typeChip} ${chipClass}`}>{issue.type}</span>
        <span className={styles.desc}>{descriptionSnippet(issue.description)}</span>
        {jobSummaries && jobSummaries.length > 0 && (
          <span
            className={styles.jobIndicator}
            onClick={(e) => e.stopPropagation()}
            role="presentation"
          >
            <JobStatusIndicator jobs={jobSummaries} onJobClick={handleJobClick} />
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
