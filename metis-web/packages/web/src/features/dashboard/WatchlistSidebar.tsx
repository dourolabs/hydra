import { useMemo } from "react";
import type { JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { descriptionSnippet } from "../../utils/text";
import { computeIssueProgress, type IssueProgress } from "./activityUtils";
import styles from "./WatchlistSidebar.module.css";

interface WatchlistSidebarProps {
  roots: IssueTreeNode[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
}

/**
 * SVG donut/ring chart showing subtask progress breakdown.
 */
function MiniDonut({ progress }: { progress: IssueProgress }) {
  const { done, active, needsAttention, open, failed, total } = progress;
  if (total === 0) return null;

  const radius = 12;
  const circumference = 2 * Math.PI * radius;
  const size = 32;

  // Build segments: done (green), active (yellow), needsAttention (orange), open (blue), failed (red)
  const segments: Array<{ value: number; color: string }> = [
    { value: done, color: "var(--color-status-closed)" },
    { value: active, color: "var(--color-status-in-progress)" },
    { value: needsAttention, color: "var(--color-status-blocked)" },
    { value: open, color: "var(--color-status-open)" },
    { value: failed, color: "var(--color-status-failed)" },
  ].filter((s) => s.value > 0);

  let offset = 0;
  const arcs = segments.map((seg, i) => {
    const length = (seg.value / total) * circumference;
    const gap = total > 1 ? 1 : 0;
    const arc = (
      <circle
        key={i}
        className={styles.donutRing}
        cx={size / 2}
        cy={size / 2}
        r={radius}
        stroke={seg.color}
        strokeDasharray={`${Math.max(0, length - gap)} ${circumference - Math.max(0, length - gap)}`}
        strokeDashoffset={-offset}
        strokeLinecap="round"
      />
    );
    offset += length;
    return arc;
  });

  return (
    <svg
      className={styles.donutContainer}
      viewBox={`0 0 ${size} ${size}`}
      width={size}
      height={size}
    >
      {arcs}
      <text className={styles.donutCenter} x={size / 2} y={size / 2}>
        {done}/{total}
      </text>
    </svg>
  );
}

function formatProgressText(p: IssueProgress): string {
  const parts: string[] = [];
  if (p.done > 0) parts.push(`${p.done}/${p.total} done`);
  if (p.active > 0) parts.push(`${p.active} active`);
  if (p.needsAttention > 0) parts.push(`${p.needsAttention} need attention`);
  if (p.failed > 0) parts.push(`${p.failed} failed`);
  if (p.open > 0) parts.push(`${p.open} open`);
  if (parts.length === 0) return "No subtasks";
  return parts.join(" \u00b7 ");
}

export function WatchlistSidebar({
  roots,
  jobsByIssue,
  activeFilter,
  onFilterChange,
}: WatchlistSidebarProps) {
  const progressList = useMemo(
    () => computeIssueProgress(roots, jobsByIssue),
    [roots, jobsByIssue],
  );

  if (progressList.length === 0) {
    return <p className={styles.empty}>No watched issues.</p>;
  }

  return (
    <div>
      <div className={styles.sidebarHeader}>Issues</div>
      <ul className={styles.sidebar}>
        {progressList.map((p) => {
          const isActive = activeFilter === p.rootId;
          const cardClasses = [styles.issueCard];
          if (isActive) cardClasses.push(styles.activeFilter);

          return (
            <li
              key={p.rootId}
              className={cardClasses.join(" ")}
              onClick={() => onFilterChange(p.rootId)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onFilterChange(p.rootId);
                }
              }}
            >
              <span className={styles.issueInfo}>
                <span className={styles.issueDesc}>
                  {descriptionSnippet(p.rootIssue.issue.description, 50)}
                </span>
                <span className={styles.issueSummary}>
                  {formatProgressText(p)}
                </span>
              </span>
              <MiniDonut progress={p} />
            </li>
          );
        })}
      </ul>
    </div>
  );
}
