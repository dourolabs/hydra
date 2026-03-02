import React, { useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, JobStatusIndicator } from "@metis/ui";
import type { JobSummaryRecord } from "@metis/api";
import type { WorkItem } from "./useTransitiveWorkItems";
import type { ItemNotificationState } from "./useItemNotifications";
import { useAuth } from "../auth/useAuth";
import { toJobSummary } from "../../utils/jobMapping";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatRelativeTime } from "../../utils/time";
import styles from "./ItemRow.module.css";

const STATUS_DOT_CLASSES: Record<string, string> = {
  open: styles.statusDotOpen,
  "in-progress": styles.statusDotInProgress,
  closed: styles.statusDotClosed,
  failed: styles.statusDotFailed,
  dropped: styles.statusDotDropped,
  blocked: styles.statusDotBlocked,
  rejected: styles.statusDotRejected,
};

function PatchIcon() {
  return (
    <svg
      className={styles.typeIcon}
      width="16"
      height="16"
      viewBox="0 0 20 20"
      fill="currentColor"
    >
      <path d="M3 4a2 2 0 1 1 4 0 2 2 0 0 1-4 0zM3 16a2 2 0 1 1 4 0 2 2 0 0 1-4 0zM13 4a2 2 0 1 1 4 0 2 2 0 0 1-4 0zM4 6h2v8H4zM14 6.5C14 10 10 13 6 14V12C9 11 12 9 12 6.5H14Z" />
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

const TYPE_ICONS: Record<WorkItem["kind"], (() => React.JSX.Element) | null> = {
  issue: null,
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

  // Status dot (issues only)
  const badgeStatus = item.kind === "issue"
    ? issueToBadgeStatus(item.data.issue.status)
    : undefined;

  // Patch display status
  let patchDisplayStatus: string | undefined;
  if (item.kind === "patch") {
    const { status, review_summary } = item.data.patch;
    if (status === "Merged") {
      patchDisplayStatus = "Merged";
    } else if (status === "ChangesRequested") {
      patchDisplayStatus = "Changes Requested";
    } else if (status === "Open" && review_summary.approved) {
      patchDisplayStatus = "Approved";
    } else if (status === "Open") {
      patchDisplayStatus = "Open";
    } else if (status === "Closed") {
      patchDisplayStatus = "Closed";
    } else {
      patchDisplayStatus = status;
    }
  }

  // Patch GitHub PR link
  let patchPrUrl: string | undefined;
  let patchPrNumber: bigint | undefined;
  if (item.kind === "patch" && item.data.patch.github) {
    const gh = item.data.patch.github;
    patchPrUrl = gh.url ?? `https://github.com/${gh.owner}/${gh.repo}/pull/${gh.number}`;
    patchPrNumber = gh.number;
  }

  // Assignee (issues only)
  const assignee = item.kind === "issue" ? item.data.issue.assignee : undefined;

  // Highlight open issues assigned to the current user
  const currentUsername = user?.actor.type === "user" ? user.actor.username : user?.actor.creator;
  const isAssignedToMe =
    item.kind === "issue" && !item.isTerminal && !!assignee && assignee === currentUsername;

  // Job status (issues only)
  const jobSummaries = item.kind === "issue" && jobs ? jobs.map(toJobSummary) : undefined;
  const hasRunningJob = jobs?.some((j) => j.task.status === "running" || j.task.status === "pending") ?? false;

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
      {badgeStatus && (
        <span
          className={`${styles.statusDot} ${hasRunningJob ? styles.statusDotPulsing : (STATUS_DOT_CLASSES[badgeStatus] ?? "")}`}
        />
      )}
      {Icon && (
        <span className={styles.icon}>
          <Icon />
        </span>
      )}
      <span className={styles.titleGroup}>
        <span className={isUnread ? styles.titleUnread : styles.title}>
          {title}
        </span>
      </span>
      {jobSummaries && jobSummaries.length > 0 && (
        <span
          className={styles.jobIndicator}
          onClick={(e) => e.stopPropagation()}
          role="presentation"
        >
          <JobStatusIndicator jobs={jobSummaries} />
        </span>
      )}
      {patchDisplayStatus && (
        <span className={`${styles.patchBadge} ${styles[`patchBadge${patchDisplayStatus.replace(/\s+/g, "")}`] ?? ""}`}>
          {patchDisplayStatus}
        </span>
      )}
      {patchPrUrl && (
        <a
          className={styles.prLink}
          href={patchPrUrl}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(e) => e.stopPropagation()}
        >
          #{String(patchPrNumber)}
          <svg
            className={styles.externalLinkIcon}
            width="12"
            height="12"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M6 2H2v12h12v-4" />
            <path d="M9 1h6v6" />
            <path d="M15 1L7 9" />
          </svg>
        </a>
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
