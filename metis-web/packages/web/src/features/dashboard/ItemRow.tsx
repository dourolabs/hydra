import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Avatar, Badge } from "@metis/ui";
import type { JobSummaryRecord, LabelSummary, PatchSummaryRecord } from "@metis/api";
import type { ChildStatus } from "./computeIssueProgress";
import type { WorkItem } from "./useTransitiveWorkItems";
import { StatusBoxes } from "./StatusBoxes";
import { useAuth } from "../auth/useAuth";
import { apiClient } from "../../api/client";
import { normalizeIssueStatus, normalizePatchStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatDuration } from "../../utils/time";
import { LabelChip } from "../labels/LabelChip";
import { useSwipeToArchive } from "./useSwipeToArchive";
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
  childStatuses?: ChildStatus[];
  isActive?: boolean;
  filterRootId?: string | null;
  inboxLabelId?: string;
  patchMap?: Map<string, PatchSummaryRecord>;
}

export function ItemRow({ item, jobs, childStatuses, isActive, filterRootId, inboxLabelId, patchMap }: ItemRowProps) {
  const navigate = useNavigate();
  const { user } = useAuth();
  const queryClient = useQueryClient();

  const archiveMutation = useMutation({
    mutationFn: (issueId: string) =>
      apiClient.removeLabelFromObject(inboxLabelId!, issueId),
    onMutate: async (issueId) => {
      await queryClient.cancelQueries({ queryKey: ["issues"] });
      const previousQueries = queryClient.getQueriesData({ queryKey: ["issues"] });
      queryClient.setQueriesData<{ issues: Array<{ issue_id: string; issue: { labels?: Array<{ label_id: string }> } }> }>(
        { queryKey: ["issues"] },
        (old) => {
          if (!old) return old;
          return {
            ...old,
            issues: old.issues.map((issue) =>
              issue.issue_id === issueId
                ? {
                    ...issue,
                    issue: {
                      ...issue.issue,
                      labels: (issue.issue.labels ?? []).filter(
                        (l) => l.label_id !== inboxLabelId,
                      ),
                    },
                  }
                : issue,
            ),
          };
        },
      );
      return { previousQueries };
    },
    onError: (_err, _issueId, context) => {
      if (context?.previousQueries) {
        for (const [key, data] of context.previousQueries) {
          queryClient.setQueryData(key, data);
        }
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });
  const Icon = TYPE_ICONS[item.kind];

  const handleClick = useCallback(() => {
    const paths: Record<WorkItem["kind"], string> = {
      issue: `/issues/${item.id}`,
      patch: `/patches/${item.id}`,
      document: `/documents/${item.id}`,
    };
    const params = new URLSearchParams({ from: "dashboard" });
    params.set("filter", filterRootId ?? "everything");
    navigate(`${paths[item.kind]}?${params.toString()}`);
  }, [navigate, item, filterRootId]);

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
    title = item.data.issue.title || descriptionSnippet(item.data.issue.description);
  } else if (item.kind === "patch") {
    title = item.data.patch.title || item.id;
  } else {
    title = item.data.document.title || item.data.document.path || item.data.document_id;
  }

  // Status dot (issues only)
  const badgeStatus = item.kind === "issue"
    ? normalizeIssueStatus(item.data.issue.status)
    : undefined;

  // Patch badge status
  const patchBadgeStatus = item.kind === "patch"
    ? (item.data.patch.status === "Open" && item.data.patch.review_summary.approved
        ? "approved"
        : normalizePatchStatus(item.data.patch.status))
    : undefined;

  // Patch GitHub PR link
  let patchPrUrl: string | undefined;
  let patchPrNumber: bigint | undefined;
  if (item.kind === "patch" && item.data.patch.github) {
    const gh = item.data.patch.github;
    patchPrUrl = gh.url ?? `https://github.com/${gh.owner}/${gh.repo}/pull/${gh.number}`;
    patchPrNumber = gh.number;
  }

  // Issue PR/patch link for review-request and merge-request issues
  let issuePrUrl: string | undefined;
  let issuePrLabel: string | undefined;
  if (
    item.kind === "issue" &&
    (item.data.issue.type === "review-request" || item.data.issue.type === "merge-request") &&
    item.data.issue.patches.length > 0 &&
    patchMap
  ) {
    const firstPatchId = item.data.issue.patches[0];
    const patchRecord = patchMap.get(firstPatchId);
    if (patchRecord?.patch.github) {
      const gh = patchRecord.patch.github;
      issuePrUrl = gh.url ?? `https://github.com/${gh.owner}/${gh.repo}/pull/${gh.number}`;
      issuePrLabel = `${gh.owner}/${gh.repo}#${gh.number}`;
    } else if (firstPatchId) {
      issuePrUrl = `/patches/${firstPatchId}`;
      issuePrLabel = firstPatchId;
    }
  }

  // Assignee (issues only)
  const assignee = item.kind === "issue" ? item.data.issue.assignee : undefined;

  // Highlight open issues assigned to the current user
  const currentUsername = user?.actor.type === "user" ? user.actor.username : user?.actor.creator;
  const isAssignedToMe =
    item.kind === "issue" && !item.isTerminal && !!assignee && assignee === currentUsername;

  // Job status (issues only) — isActive is tree-computed, fall back to direct job check
  const hasRunningJob = isActive ?? (jobs?.some((j) => j.task.status === "running" || j.task.status === "pending") ?? false);

  // Job duration display
  const runningJob = useMemo(
    () => jobs?.find((j) => j.task.status === "running" || j.task.status === "pending"),
    [jobs],
  );
  const lastFinishedJob = useMemo(() => {
    if (runningJob || !jobs) return undefined;
    return jobs
      .filter((j) => j.task.status === "complete" || j.task.status === "failed")
      .sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())[0];
  }, [jobs, runningJob]);

  const [elapsed, setElapsed] = useState(() => {
    if (!runningJob?.task.start_time) return 0;
    return Date.now() - new Date(runningJob.task.start_time).getTime();
  });

  useEffect(() => {
    if (!runningJob?.task.start_time) return;
    setElapsed(Date.now() - new Date(runningJob.task.start_time).getTime());
    const id = setInterval(() => {
      setElapsed(Date.now() - new Date(runningJob.task.start_time!).getTime());
    }, 1000);
    return () => clearInterval(id);
  }, [runningJob]);

  let durationText: string;
  let durationClass: string;
  if (runningJob) {
    durationText = formatDuration(elapsed);
    durationClass = `${styles.timestamp} ${styles.timerRunning}`;
  } else if (lastFinishedJob?.task.start_time && lastFinishedJob.task.end_time) {
    durationText = formatDuration(
      new Date(lastFinishedJob.task.end_time).getTime() - new Date(lastFinishedJob.task.start_time).getTime(),
    );
    durationClass = styles.timestamp;
  } else {
    durationText = "\u2014";
    durationClass = styles.timestamp;
  }

  const rowClasses = [styles.row];
  if (item.isTerminal) rowClasses.push(styles.terminal);

  // Labels (issues only) — filter out hidden labels
  const allLabels = item.kind === "issue" && item.data.issue.labels && item.data.issue.labels.length > 0
    ? item.data.issue.labels.filter((l: LabelSummary) => !l.hidden)
    : null;

  const showArchive = !!inboxLabelId && item.kind === "issue";

  const rowRef = useRef<HTMLLIElement>(null);
  const handleArchiveSwipe = useCallback(() => {
    archiveMutation.mutate(item.id);
  }, [archiveMutation, item.id]);
  useSwipeToArchive(rowRef, { onArchive: handleArchiveSwipe, enabled: showArchive });

  return (
    <li
      ref={rowRef}
      className={rowClasses.join(" ")}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      role="button"
      tabIndex={0}
    >
      {badgeStatus && (
        <span
          className={`${styles.statusDot} ${hasRunningJob ? styles.statusDotPulsing : isAssignedToMe ? styles.statusDotAttention : (STATUS_DOT_CLASSES[badgeStatus] ?? "")}`}
        />
      )}
      {Icon && (
        <span className={styles.icon}>
          <Icon />
        </span>
      )}
      <span className={styles.titleGroup}>
        <span className={styles.title}>
          {title}
        </span>
        {item.kind === "issue" && item.data.issue.progress && (
          <span className={styles.progressLine}>
            {item.data.issue.progress}
          </span>
        )}
      </span>
      {allLabels && (
        <span className={styles.labels}>
          {allLabels.map((label: LabelSummary) => (
            <LabelChip
              key={label.label_id}
              name={label.name}
              color={label.color}
            />
          ))}
        </span>
      )}
      {patchBadgeStatus && (
        <Badge status={patchBadgeStatus} />
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
      {issuePrUrl && (
        <a
          className={styles.prLink}
          href={issuePrUrl}
          target={issuePrUrl.startsWith("http") ? "_blank" : undefined}
          rel={issuePrUrl.startsWith("http") ? "noopener noreferrer" : undefined}
          onClick={(e) => {
            e.stopPropagation();
            if (!issuePrUrl!.startsWith("http")) {
              e.preventDefault();
              navigate(issuePrUrl!);
            }
          }}
        >
          {issuePrLabel}
          {issuePrUrl.startsWith("http") && (
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
          )}
        </a>
      )}
      {assignee && (
        <span className={styles.assignee}>
          <Avatar name={assignee} size="sm" />
        </span>
      )}
      {showArchive && (
        <button
          type="button"
          className={styles.archiveButton}
          title="Archive"
          onClick={(e) => {
            e.stopPropagation();
            archiveMutation.mutate(item.id);
          }}
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M2 4h12v2H2zM3 6v7a1 1 0 001 1h8a1 1 0 001-1V6" />
            <path d="M6 9h4" />
          </svg>
        </button>
      )}
      <span className={styles.rightColumn}>
        <span className={durationClass}>
          {durationText}
        </span>
        {childStatuses && childStatuses.length > 0 && (
          <StatusBoxes children={childStatuses} />
        )}
      </span>
    </li>
  );
}
