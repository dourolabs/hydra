import React, { useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, Badge, JobStatusIndicator } from "@metis/ui";
import type { JobSummaryRecord } from "@metis/api";
import type { WorkItem } from "./useTransitiveWorkItems";
import { toJobSummary } from "../../utils/jobMapping";
import {
  issueToBadgeStatus,
  patchToBadgeStatus,
} from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatRelativeTime } from "../../utils/time";
import styles from "./ItemRow.module.css";

function IssueIcon() {
  return (
    <svg
      className={styles.typeIcon}
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="currentColor"
    >
      <circle
        cx="8"
        cy="8"
        r="6.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <circle cx="8" cy="8" r="2" />
    </svg>
  );
}

function PatchIcon() {
  return (
    <svg
      className={styles.typeIcon}
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M10 1L6 15M4 4L1 8l3 4M12 4l3 4-3 4" />
    </svg>
  );
}

function DocumentIcon() {
  return (
    <svg
      className={styles.typeIcon}
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M9 1H4a1 1 0 00-1 1v12a1 1 0 001 1h8a1 1 0 001-1V5L9 1z" />
      <path d="M9 1v4h4" />
      <path d="M5 8h6M5 11h4" />
    </svg>
  );
}

const TYPE_ICONS: Record<WorkItem["kind"], () => React.JSX.Element> = {
  issue: IssueIcon,
  patch: PatchIcon,
  document: DocumentIcon,
};

interface ItemRowProps {
  item: WorkItem;
  jobs?: JobSummaryRecord[];
}

export function ItemRow({ item, jobs }: ItemRowProps) {
  const navigate = useNavigate();
  const Icon = TYPE_ICONS[item.kind];

  const handleClick = useCallback(() => {
    const paths: Record<WorkItem["kind"], string> = {
      issue: `/issues/${item.id}`,
      patch: `/patches/${item.id}`,
      document: `/documents/${item.id}`,
    };
    navigate(`${paths[item.kind]}?from=dashboard`);
  }, [navigate, item.kind, item.id]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        handleClick();
      }
    },
    [handleClick],
  );

  // Title
  let title: string;
  if (item.kind === "issue") {
    title = descriptionSnippet(item.data.issue.description);
  } else if (item.kind === "patch") {
    title = item.data.patch.title || descriptionSnippet(item.data.patch.description);
  } else {
    title = item.data.document.title || item.data.document.path || item.data.document_id;
  }

  // Status badge
  let badgeStatus;
  let statusLabel: string | undefined;
  if (item.kind === "issue") {
    badgeStatus = issueToBadgeStatus(item.data.issue.status);
    statusLabel = item.data.issue.status;
  } else if (item.kind === "patch") {
    badgeStatus = patchToBadgeStatus(item.data.patch.status);
    statusLabel = item.data.patch.status;
  }

  // Assignee
  let assignee: string | null | undefined;
  if (item.kind === "issue") {
    assignee = item.data.issue.assignee;
  } else if (item.kind === "patch") {
    assignee = item.data.patch.creator;
  }

  // Job status (issues only)
  const jobSummaries = item.kind === "issue" && jobs ? jobs.map(toJobSummary) : undefined;

  const rowClasses = [styles.row];
  if (item.isTerminal) rowClasses.push(styles.terminal);

  return (
    <li
      className={rowClasses.join(" ")}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      role="button"
      tabIndex={0}
    >
      <span className={styles.icon}>
        <Icon />
      </span>
      <span className={styles.title}>{title}</span>
      {badgeStatus && (
        <span className={styles.status}>
          <Badge status={badgeStatus} />
          {statusLabel && <span className={styles.statusLabel}>{statusLabel}</span>}
        </span>
      )}
      {jobSummaries && jobSummaries.length > 0 && (
        <span
          className={styles.jobIndicator}
          onClick={(e) => e.stopPropagation()}
          role="presentation"
        >
          <JobStatusIndicator jobs={jobSummaries} />
        </span>
      )}
      {assignee && (
        <span className={styles.assignee}>
          <Avatar name={assignee} size="sm" />
        </span>
      )}
      <span className={styles.timestamp}>
        {formatRelativeTime(item.lastUpdated)}
      </span>
    </li>
  );
}
