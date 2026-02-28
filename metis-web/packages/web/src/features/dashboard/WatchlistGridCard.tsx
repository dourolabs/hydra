import { useState, useMemo, useCallback } from "react";
import { Badge } from "@metis/ui";
import type { IssueType, JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import {
  computeCardMetrics,
  flattenSubtasks,
  type StatusPill,
} from "./gridUtils";
import styles from "./WatchlistGridCard.module.css";

const typeChipClass: Record<IssueType, string> = {
  task: styles.typeTask,
  bug: styles.typeBug,
  feature: styles.typeFeature,
  chore: styles.typeChore,
  "merge-request": styles.typeMergeRequest,
  "review-request": styles.typeReviewRequest,
  unknown: styles.typeUnknown,
};

const pillClass: Record<StatusPill["kind"], string> = {
  active: styles.pillActive,
  review: styles.pillReview,
  failed: styles.pillFailed,
  queued: styles.pillQueued,
  complete: styles.pillComplete,
  "on-track": styles.pillOnTrack,
};

interface WatchlistGridCardProps {
  root: IssueTreeNode;
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
}

export function WatchlistGridCard({
  root,
  jobsByIssue,
  selectedId,
  onSelect,
}: WatchlistGridCardProps) {
  const [expanded, setExpanded] = useState(false);

  const metrics = useMemo(
    () => computeCardMetrics(root, jobsByIssue),
    [root, jobsByIssue],
  );

  const subtasks = useMemo(
    () => (expanded ? flattenSubtasks(root, jobsByIssue) : []),
    [expanded, root, jobsByIssue],
  );

  const handleCardClick = useCallback(() => {
    onSelect(root.id);
  }, [onSelect, root.id]);

  const handleExpandToggle = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      setExpanded((prev) => !prev);
    },
    [],
  );

  const handleSubtaskClick = useCallback(
    (e: React.MouseEvent, subtaskId: string) => {
      e.stopPropagation();
      onSelect(subtaskId);
    },
    [onSelect],
  );

  const { issue } = root.issue;
  const isSelected = root.id === selectedId;
  const cardClass = isSelected
    ? `${styles.card} ${styles.selected}`
    : styles.card;

  return (
    <div
      className={cardClass}
      onClick={handleCardClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          handleCardClick();
        }
      }}
    >
      {/* Title line */}
      <div className={styles.titleLine}>
        <span className={`${styles.typeChip} ${typeChipClass[issue.type]}`}>
          {issue.type}
        </span>
        <span className={styles.title}>
          {descriptionSnippet(issue.description)}
        </span>
      </div>

      {/* Progress bar */}
      {metrics.total > 0 && (
        <div className={styles.progressRow}>
          <div className={styles.progressBar}>
            {metrics.done > 0 && (
              <div
                className={`${styles.progressSegment} ${styles.segmentDone}`}
                style={{ width: `${(metrics.done / metrics.total) * 100}%` }}
              />
            )}
            {metrics.active > 0 && (
              <div
                className={`${styles.progressSegment} ${styles.segmentActive}`}
                style={{ width: `${(metrics.active / metrics.total) * 100}%` }}
              />
            )}
            {metrics.review > 0 && (
              <div
                className={`${styles.progressSegment} ${styles.segmentReview}`}
                style={{ width: `${(metrics.review / metrics.total) * 100}%` }}
              />
            )}
            {metrics.failed > 0 && (
              <div
                className={`${styles.progressSegment} ${styles.segmentFailed}`}
                style={{ width: `${(metrics.failed / metrics.total) * 100}%` }}
              />
            )}
            {metrics.open > 0 && (
              <div
                className={`${styles.progressSegment} ${styles.segmentOpen}`}
                style={{ width: `${(metrics.open / metrics.total) * 100}%` }}
              />
            )}
          </div>
          <span className={styles.progressText}>
            {metrics.done}/{metrics.total} done
          </span>
        </div>
      )}

      {/* Status pills */}
      <div className={styles.pillRow}>
        {metrics.pills.map((pill) => (
          <span
            key={pill.kind}
            className={`${styles.pill} ${pillClass[pill.kind]}`}
          >
            {pill.kind === "active" && <span className={styles.pulse} />}
            {pill.label}
          </span>
        ))}
      </div>

      {/* Expand toggle */}
      {metrics.total > 0 && (
        <button
          type="button"
          className={styles.expandToggle}
          onClick={handleExpandToggle}
        >
          {expanded ? "hide details" : "show details"}
        </button>
      )}

      {/* Expanded subtask list */}
      {expanded && subtasks.length > 0 && (
        <ul className={styles.subtaskList}>
          {subtasks.map((st) => (
            <li
              key={st.id}
              className={styles.subtaskItem}
              onClick={(e) => handleSubtaskClick(e, st.id)}
              style={{ paddingLeft: `${st.depth * 12 + 4}px` }}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onSelect(st.id);
                }
              }}
            >
              <Badge status={issueToBadgeStatus(st.issue.issue.issue.status)} />
              <span className={styles.subtaskDesc}>
                {descriptionSnippet(st.issue.issue.issue.description)}
              </span>
              {st.hasRunningJob && <span className={styles.subtaskJobDot} />}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
