import { useCallback } from "react";
import { Avatar, Badge, JobStatusIndicator } from "@metis/ui";
import type { JobSummary } from "@metis/ui";
import type { IssueVersionRecord, JobVersionRecord } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import styles from "./IssueRow.module.css";

interface IssueRowProps {
  record: IssueVersionRecord;
  dimmed?: boolean;
  jobs?: JobVersionRecord[];
  onJobClick?: (issueId: string, jobId: string) => void;
}

function toJobSummary(record: JobVersionRecord): JobSummary {
  const status = record.task.status === "unknown" ? "created" : record.task.status;
  return {
    jobId: record.job_id,
    status,
    startTime: record.task.start_time,
    endTime: record.task.end_time,
  };
}

export function IssueRow({ record, dimmed, jobs, onJobClick }: IssueRowProps) {
  const { issue } = record;

  const handleJobClick = useCallback(
    (jobId: string) => {
      onJobClick?.(record.issue_id, jobId);
    },
    [onJobClick, record.issue_id],
  );

  const jobSummaries = jobs?.map(toJobSummary);

  const blockedDep = issue.dependencies?.find((d) => d.type === "blocked-on");

  const classNames = [styles.row];
  if (dimmed) classNames.push(styles.dimmed);
  if (blockedDep) classNames.push(styles.blocked);

  return (
    <span className={classNames.join(" ")}>
      <span className={styles.topRow}>
        <Badge status={issueToBadgeStatus(issue.status)} />
        {jobSummaries && jobSummaries.length > 0 && (
          <span
            className={styles.jobIndicator}
            onClick={(e) => e.stopPropagation()}
            role="presentation"
          >
            <JobStatusIndicator jobs={jobSummaries} onJobClick={handleJobClick} />
          </span>
        )}
        <span className={styles.id}>{record.issue_id}</span>
        {issue.assignee && <Avatar name={issue.assignee} size="sm" />}
        {blockedDep && (
          <span className={styles.blockedLabel} title={`Blocked by ${blockedDep.issue_id}`}>
            blocked by {blockedDep.issue_id}
          </span>
        )}
      </span>
      <span className={styles.desc}>{descriptionSnippet(issue.description)}</span>
    </span>
  );
}
