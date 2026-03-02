import React, { useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, Badge, JobStatusIndicator } from "@metis/ui";
import type { JobSummaryRecord } from "@metis/api";
import type { WorkItem } from "./useTransitiveWorkItems";
import type { ItemNotificationState } from "./useItemNotifications";
import { useAuth } from "../auth/useAuth";
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
  notification?: ItemNotificationState;
  onMarkRead?: (item: WorkItem) => Promise<void> | void;
  filterRootId?: string | null;
}

export function ItemRow({ item, jobs, notification, onMarkRead, filterRootId }: ItemRowProps) {
  const navigate = useNavigate();
  const { user } = useAuth();
  const Icon = TYPE_ICONS[item.kind];

  const handleClick = useCallback(async () => {
    if (notification?.unread && onMarkRead) {
      try {
        await onMarkRead(item);
      } catch {
        // Mark-as-read failed; still navigate
      }
    }
    const paths: Record<WorkItem["kind"], string> = {
      issue: `/issues/${item.id}`,
      patch: `/patches/${item.id}`,
      document: `/documents/${item.id}`,
    };
    const params = new URLSearchParams({ from: "dashboard" });
    if (filterRootId) params.set("filter", filterRootId);
    navigate(`${paths[item.kind]}?${params.toString()}`);
  }, [navigate, item, notification, onMarkRead, filterRootId]);

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
    title = item.data.patch.title || item.id;
  } else {
    title = item.data.document.title || item.data.document.path || item.data.document_id;
  }

  // Status badge
  let badgeStatus;
  if (item.kind === "issue") {
    badgeStatus = issueToBadgeStatus(item.data.issue.status);
  } else if (item.kind === "patch") {
    badgeStatus = patchToBadgeStatus(item.data.patch.status);
  }

  // Assignee
  let assignee: string | null | undefined;
  if (item.kind === "issue") {
    assignee = item.data.issue.assignee;
  } else if (item.kind === "patch") {
    assignee = item.data.patch.creator;
  }

  // Highlight open issues assigned to the current user
  const currentUsername = user?.actor.type === "user" ? user.actor.username : user?.actor.creator;
  const isAssignedToMe =
    item.kind === "issue" && !item.isTerminal && !!assignee && assignee === currentUsername;

  // Job status (issues only)
  const jobSummaries = item.kind === "issue" && jobs ? jobs.map(toJobSummary) : undefined;

  const isUnread = notification?.unread ?? false;
  const rowClasses = [styles.row];
  if (item.isTerminal) rowClasses.push(styles.terminal);
  if (isUnread) rowClasses.push(styles.unread);
  if (isAssignedToMe) rowClasses.push(styles.assignedToMe);

  return (
    <li
      className={rowClasses.join(" ")}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      role="button"
      tabIndex={0}
    >
      {isUnread && <span className={styles.unreadDot} />}
      <span className={styles.icon}>
        <Icon />
      </span>
      <span className={styles.titleGroup}>
        <span className={isUnread ? styles.titleUnread : styles.title}>
          {title}
        </span>
        {isUnread && notification?.latestSummary && (
          <span className={styles.notificationSummary}>
            {notification.latestSummary}
          </span>
        )}
      </span>
      {badgeStatus && (
        <span className={styles.status}>
          <Badge status={badgeStatus} />
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
