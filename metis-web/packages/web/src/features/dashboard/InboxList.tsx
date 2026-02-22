import { Badge } from "@metis/ui";
import type { IssueSummaryRecord } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatRelativeTime } from "../../utils/time";
import styles from "./InboxList.module.css";

interface InboxListProps {
  issues: IssueSummaryRecord[];
  selectedId: string | null;
  onSelect: (issueId: string) => void;
}

export function InboxList({ issues, selectedId, onSelect }: InboxListProps) {
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
              <div className={styles.top}>
                <Badge status={issueToBadgeStatus(record.issue.status)} />
                <span className={styles.desc}>
                  {descriptionSnippet(record.issue.description, 60)}
                </span>
              </div>
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
