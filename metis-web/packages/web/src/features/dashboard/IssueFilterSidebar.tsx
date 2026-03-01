import { useMemo } from "react";
import type { JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { descriptionSnippet } from "../../utils/text";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { computeIssueProgress, type IssueProgress } from "./activityUtils";
import { writeCollapsed } from "./sidebarStorage";
import styles from "./IssueFilterSidebar.module.css";

interface IssueFilterSidebarProps {
  roots: IssueTreeNode[];
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
  collapsed: boolean;
  onToggleCollapsed: (collapsed: boolean) => void;
  jobsByIssue: Map<string, JobSummaryRecord[]>;
}

/** Small SVG donut ring showing completed / in-progress / open segments. */
function ProgressCircle({ progress }: { progress: IssueProgress }) {
  const size = 24;
  const strokeWidth = 3;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;

  if (progress.total === 0) {
    return (
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        className={progress.hasActive ? styles.pulse : undefined}
      >
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--color-text-tertiary)"
          strokeWidth={strokeWidth}
        />
      </svg>
    );
  }

  const closedFrac = progress.closed / progress.total;
  const inProgressFrac = progress.inProgress / progress.total;
  const openFrac = progress.open / progress.total;

  const closedLen = closedFrac * circumference;
  const inProgressLen = inProgressFrac * circumference;
  const openLen = openFrac * circumference;

  // Each segment: dasharray = "segment gap", dashoffset rotates start position.
  // Rotation starts at 12 o'clock (-90deg). Segments go: closed, inProgress, open.
  const closedOffset = 0;
  const inProgressOffset = -(closedLen);
  const openOffset = -(closedLen + inProgressLen);

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className={progress.hasActive ? styles.pulse : undefined}
      style={{ transform: "rotate(-90deg)" }}
    >
      {closedLen > 0 && (
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--color-status-closed)"
          strokeWidth={strokeWidth}
          strokeDasharray={`${closedLen} ${circumference - closedLen}`}
          strokeDashoffset={closedOffset}
        />
      )}
      {inProgressLen > 0 && (
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--color-status-in-progress)"
          strokeWidth={strokeWidth}
          strokeDasharray={`${inProgressLen} ${circumference - inProgressLen}`}
          strokeDashoffset={inProgressOffset}
        />
      )}
      {openLen > 0 && (
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--color-text-tertiary)"
          strokeWidth={strokeWidth}
          strokeDasharray={`${openLen} ${circumference - openLen}`}
          strokeDashoffset={openOffset}
        />
      )}
    </svg>
  );
}

export function IssueFilterSidebar({
  roots,
  activeFilter,
  onFilterChange,
  collapsed,
  onToggleCollapsed,
  jobsByIssue,
}: IssueFilterSidebarProps) {
  const progressList = useMemo(() => {
    const list = computeIssueProgress(roots, jobsByIssue);
    return list.sort((a, b) => {
      const aInactive = TERMINAL_STATUSES.has(a.rootIssue.issue.status) ? 1 : 0;
      const bInactive = TERMINAL_STATUSES.has(b.rootIssue.issue.status) ? 1 : 0;
      if (aInactive !== bInactive) return aInactive - bInactive;
      return (
        new Date(b.rootIssue.creation_time).getTime() -
        new Date(a.rootIssue.creation_time).getTime()
      );
    });
  }, [roots, jobsByIssue]);

  if (progressList.length === 0) return null;

  return (
    <div className={`${styles.sidebar} ${collapsed ? styles.collapsed : ""}`}>
      <div className={styles.header}>
        {!collapsed && <span className={styles.title}>Issues</span>}
        <button
          type="button"
          className={styles.toggle}
          onClick={() => {
            const next = !collapsed;
            writeCollapsed(next);
            onToggleCollapsed(next);
          }}
          aria-label={collapsed ? "Expand filter sidebar" : "Collapse filter sidebar"}
        >
          {collapsed ? "\u25B6" : "\u25C0"}
        </button>
      </div>
      <ul className={`${styles.list} ${collapsed ? styles.listCollapsed : ""}`}>
        <li
          className={`${styles.item} ${activeFilter === null ? styles.active : ""}`}
          onClick={() => onFilterChange(null)}
          role="button"
          tabIndex={collapsed ? -1 : 0}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              onFilterChange(null);
            }
          }}
        >
          <span className={styles.itemLabel}>All issues</span>
        </li>
        {progressList.map((p) => {
          const label = descriptionSnippet(p.rootIssue.issue.description, 40);
          const isActive = activeFilter === p.rootId;
          return (
            <li
              key={p.rootId}
              className={`${styles.item} ${isActive ? styles.active : ""}`}
              onClick={() => onFilterChange(p.rootId)}
              role="button"
              tabIndex={collapsed ? -1 : 0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onFilterChange(p.rootId);
                }
              }}
            >
              <span className={styles.itemLabel}>{label}</span>
              <span className={styles.itemStats}>
                <ProgressCircle progress={p} />
                {p.closed}/{p.total}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
