import { Avatar, Badge } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import styles from "./IssueRow.module.css";

interface IssueRowProps {
  record: IssueVersionRecord;
  dimmed?: boolean;
}

export function IssueRow({ record, dimmed }: IssueRowProps) {
  const { issue } = record;
  return (
    <span className={`${styles.row}${dimmed ? ` ${styles.dimmed}` : ""}`}>
      <Badge status={issueToBadgeStatus(issue.status)} />
      <span className={styles.id}>{record.issue_id}</span>
      {issue.assignee && <Avatar name={issue.assignee} size="sm" />}
      <span className={styles.desc}>{descriptionSnippet(issue.description)}</span>
    </span>
  );
}
