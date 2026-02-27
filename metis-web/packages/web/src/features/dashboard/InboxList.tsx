import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { IssueRow } from "../issues/IssueRow";
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
  if (issues.length === 0) {
    return <p className={styles.empty}>No assigned issues.</p>;
  }

  return (
    <ul className={styles.list}>
      {issues.map((record) => {
        const active = record.issue_id === selectedId;
        return (
          <li key={record.issue_id}>
            <button
              className={`${styles.item}${active ? ` ${styles.active}` : ""}`}
              onClick={() => onSelect(record.issue_id)}
              type="button"
            >
              <IssueRow
                record={record}
                jobs={jobsByIssue?.get(record.issue_id)}
                onJobClick={onJobClick}
              />
              <div className={styles.bottom}>
                <span className={styles.id}>{record.issue_id}</span>
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
