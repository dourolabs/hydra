import { useCallback } from "react";
import { Avatar, Badge, JobStatusIndicator } from "@metis/ui";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { toJobSummary } from "../../utils/jobMapping";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatRelativeTime } from "../../utils/time";
import styles from "./InboxList.module.css";

interface InboxListProps {
  issues: IssueSummaryRecord[];
  jobsByIssue?: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  onJobClick?: (issueId: string, jobId: string) => void;
}

export function InboxList({ issues, jobsByIssue, selectedId, onSelect, onJobClick }: InboxListProps) {
  const handleJobClick = useCallback(
    (issueId: string, jobId: string) => {
      onJobClick?.(issueId, jobId);
    },
    [onJobClick],
  );

  if (issues.length === 0) {
    return <p className={styles.empty}>No assigned issues.</p>;
  }

  return (
    <ul className={styles.list}>
      {issues.map((record) => {
        const active = record.issue_id === selectedId;
        const jobs = jobsByIssue?.get(record.issue_id);
        const jobSummaries = jobs?.map(toJobSummary);
        return (
          <li key={record.issue_id}>
            <button
              className={`${styles.item}${active ? ` ${styles.active}` : ""}`}
              onClick={() => onSelect(record.issue_id)}
              type="button"
            >
              <div className={styles.top}>
                <Badge status={issueToBadgeStatus(record.issue.status)} />
                <span className={styles.desc}>
                  {descriptionSnippet(record.issue.description, 60)}
                </span>
              </div>
              <div className={styles.bottom}>
                <span className={styles.id}>{record.issue_id}</span>
                {jobSummaries && jobSummaries.length > 0 && (
                  <span
                    className={styles.jobIndicator}
                    onClick={(e) => e.stopPropagation()}
                    role="presentation"
                  >
                    <JobStatusIndicator jobs={jobSummaries} onJobClick={(jobId) => handleJobClick(record.issue_id, jobId)} />
                  </span>
                )}
                {record.issue.assignee && <Avatar name={record.issue.assignee} size="sm" />}
                <span className={styles.time}>
                  {formatRelativeTime(record.timestamp)}
                </span>
              </div>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
