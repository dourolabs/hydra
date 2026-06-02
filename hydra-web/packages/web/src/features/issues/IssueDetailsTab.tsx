import { useMemo } from "react";
import { Link } from "react-router-dom";
import { Avatar, Badge, TypeChip } from "@hydra/ui";
import type { IssueVersionRecord } from "@hydra/api";
import {
  principalAvatarKind,
  principalDisplayName,
} from "../principal/formatPrincipal";
import { normalizeIssueStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { useIssue } from "./useIssue";
import { IssueLabelEditor } from "./IssueLabelEditor";
import styles from "./IssueDetailsTab.module.css";

function DepRow({ issueId }: { issueId: string }) {
  const { data: record } = useIssue(issueId);
  const title = record?.issue.title || issueId;
  return (
    <Link to={`/issues/${issueId}`} className={styles.depRow} title={title}>
      {record && <Badge status={normalizeIssueStatus(record.issue.status)} />}
      <span className={styles.depRowTitle}>{title}</span>
    </Link>
  );
}

interface IssueDetailsTabProps {
  record: IssueVersionRecord;
  onOpenStatusModal: () => void;
}

export function IssueDetailsTab({ record, onOpenStatusModal }: IssueDetailsTabProps) {
  const { issue } = record;
  const issueId = record.issue_id;
  const status = normalizeIssueStatus(issue.status);
  const settings = issue.session_settings;

  const blockedOnIds = useMemo(
    () =>
      issue.dependencies.filter((d) => d.type === "blocked-on").map((d) => d.issue_id),
    [issue.dependencies],
  );

  return (
    <div className={styles.side}>
      <div className={styles.block}>
        <span className={styles.blockLabel}>Status</span>
        <button
          type="button"
          className={styles.statusButton}
          onClick={onOpenStatusModal}
          data-testid="status-chip"
        >
          <Badge status={status} />
          <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
            <path
              fillRule="evenodd"
              d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z"
              clipRule="evenodd"
            />
          </svg>
        </button>
      </div>

      <div className={styles.block}>
        <span className={styles.blockLabel}>Assignee</span>
        {issue.assignee ? (
          <span className={styles.blockValue}>
            <Avatar
              name={principalDisplayName(issue.assignee)}
              kind={principalAvatarKind(issue.assignee)}
              size="sm"
            />
            {principalDisplayName(issue.assignee)}
          </span>
        ) : (
          <span className={`${styles.blockValue} ${styles.blockEmpty}`}>Unassigned</span>
        )}
      </div>

      <div className={styles.block}>
        <span className={styles.blockLabel}>Type</span>
        {issue.type && issue.type !== "unknown" ? (
          <TypeChip type={issue.type} />
        ) : (
          <span className={`${styles.blockValue} ${styles.blockEmpty}`}>—</span>
        )}
      </div>

      {settings?.repo_name && (
        <div className={styles.block}>
          <span className={styles.blockLabel}>Repository</span>
          <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
            {settings.repo_name}
          </span>
        </div>
      )}

      {settings?.branch && (
        <div className={styles.block}>
          <span className={styles.blockLabel}>Branch</span>
          <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
            {settings.branch}
          </span>
        </div>
      )}

      <div className={styles.block}>
        <span className={styles.blockLabel}>Created</span>
        <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
          {formatTimestamp(record.creation_time)}
        </span>
      </div>

      <div className={styles.block}>
        <span className={styles.blockLabel}>Updated</span>
        <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
          {formatTimestamp(record.timestamp)}
        </span>
      </div>

      <div className={styles.block}>
        <span className={styles.blockLabel}>Labels</span>
        <IssueLabelEditor issueId={issueId} labels={record.labels ?? []} />
      </div>

      {blockedOnIds.length > 0 && (
        <div className={styles.block}>
          <span className={styles.blockLabel}>Blocked on</span>
          <div className={styles.depList}>
            {blockedOnIds.map((id) => (
              <DepRow key={id} issueId={id} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
