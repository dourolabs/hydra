import { useMemo } from "react";
import type { IssueTreeNode } from "../issues/useIssues";
import { descriptionSnippet } from "../../utils/text";
import { computeIssueProgress, type IssueProgress } from "./activityUtils";
import styles from "./WatchlistSidebar.module.css";

interface SidebarProps {
  roots: IssueTreeNode[];
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
}

/**
 * SVG donut/ring chart with 3 segments: closed (green), in-progress (yellow), open (blue).
 */
function MiniDonut({ progress }: { progress: IssueProgress }) {
  const { closed, inProgress, open, total } = progress;
  if (total === 0) return null;

  const radius = 12;
  const circumference = 2 * Math.PI * radius;
  const size = 32;

  const segments: Array<{ value: number; color: string }> = [
    { value: closed, color: "var(--color-status-closed)" },
    { value: inProgress, color: "var(--color-status-in-progress)" },
    { value: open, color: "var(--color-status-open)" },
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
        {closed}/{total}
      </text>
    </svg>
  );
}

function formatProgressText(p: IssueProgress): string {
  const parts: string[] = [];
  if (p.closed > 0) parts.push(`${p.closed}/${p.total} done`);
  if (p.inProgress > 0) parts.push(`${p.inProgress} in-progress`);
  if (p.open > 0) parts.push(`${p.open} open`);
  if (parts.length === 0) return "No subtasks";
  return parts.join(" \u00b7 ");
}

function useProgressList(roots: IssueTreeNode[]) {
  return useMemo(() => computeIssueProgress(roots), [roots]);
}

/**
 * Desktop sidebar column showing issue chips with donut charts.
 * Hidden on mobile via CSS.
 */
export function WatchlistSidebar({
  roots,
  activeFilter,
  onFilterChange,
}: SidebarProps) {
  const progressList = useProgressList(roots);

  if (progressList.length === 0) {
    return (
      <div className={styles.desktopSidebar}>
        <p className={styles.empty}>No watched issues.</p>
      </div>
    );
  }

  return (
    <div className={styles.desktopSidebar}>
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
              onClick={() => onFilterChange(isActive ? null : p.rootId)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onFilterChange(isActive ? null : p.rootId);
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

/**
 * Mobile pill bar showing issue filter chips.
 * Hidden on desktop via CSS.
 */
export function WatchlistMobilePills({
  roots,
  activeFilter,
  onFilterChange,
}: SidebarProps) {
  const progressList = useProgressList(roots);

  if (progressList.length === 0) return null;

  return (
    <div className={styles.mobilePillBar}>
      {progressList.map((p) => {
        const isActive = activeFilter === p.rootId;
        const pillClasses = [styles.pill];
        if (isActive) pillClasses.push(styles.pillActive);

        return (
          <button
            key={p.rootId}
            type="button"
            className={pillClasses.join(" ")}
            onClick={() => onFilterChange(isActive ? null : p.rootId)}
          >
            <span className={styles.pillText}>
              {descriptionSnippet(p.rootIssue.issue.description, 25)}
            </span>
            <span className={styles.pillProgress}>
              {p.closed}/{p.total}
            </span>
          </button>
        );
      })}
    </div>
  );
}
