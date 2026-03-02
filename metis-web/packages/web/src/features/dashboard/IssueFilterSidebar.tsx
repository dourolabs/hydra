import { useMemo, useState } from "react";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { descriptionSnippet } from "../../utils/text";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { computeIssueProgress, type IssueProgress } from "./computeIssueProgress";
import styles from "./IssueFilterSidebar.module.css";

interface IssueFilterSidebarProps {
  roots: IssueTreeNode[];
  allIssues: IssueSummaryRecord[];
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
  collapsed: boolean;
  drawerOpen: boolean;
  onDrawerClose: () => void;
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  username: string;
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
  allIssues,
  activeFilter,
  onFilterChange,
  collapsed,
  drawerOpen,
  onDrawerClose,
  jobsByIssue,
  username,
}: IssueFilterSidebarProps) {
  /** On mobile, selecting an issue should also close the drawer. */
  const handleFilterChange = (rootId: string | null) => {
    onFilterChange(rootId);
    onDrawerClose();
  };

  const inboxCount = useMemo(() => {
    return allIssues.filter(
      (issue) =>
        !TERMINAL_STATUSES.has(issue.issue.status) &&
        issue.issue.assignee === username,
    ).length;
  }, [allIssues, username]);

  const progressList = useMemo(() => {
    const list = computeIssueProgress(roots, jobsByIssue, username);
    return list.sort((a, b) => {
      const aInactive = TERMINAL_STATUSES.has(a.rootIssue.issue.status) ? 1 : 0;
      const bInactive = TERMINAL_STATUSES.has(b.rootIssue.issue.status) ? 1 : 0;
      if (aInactive !== bInactive) return aInactive - bInactive;
      return (
        new Date(b.rootIssue.creation_time).getTime() -
        new Date(a.rootIssue.creation_time).getTime()
      );
    });
  }, [roots, jobsByIssue, username]);

  const activeList = useMemo(
    () => progressList.filter((p) => !TERMINAL_STATUSES.has(p.rootIssue.issue.status)),
    [progressList],
  );
  const completedList = useMemo(
    () => progressList.filter((p) => TERMINAL_STATUSES.has(p.rootIssue.issue.status)),
    [progressList],
  );

  const [completedExpanded, setCompletedExpanded] = useState(false);

  if (progressList.length === 0) return null;

  const renderItem = (p: IssueProgress) => {
    const label = descriptionSnippet(p.rootIssue.issue.description, 80);
    const isActive = activeFilter === p.rootId;
    return (
      <li
        key={p.rootId}
        className={`${styles.item} ${isActive ? styles.active : ""}`}
        onClick={() => handleFilterChange(p.rootId)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleFilterChange(p.rootId);
          }
        }}
      >
        <span className={styles.itemLabel}>{label}</span>
        <span className={styles.itemRight}>
          {p.needsAttentionCount > 0 && (
            <span className={styles.needsAttentionChip}>
              {p.needsAttentionCount}
            </span>
          )}
          <span className={styles.itemStats}>
            <ProgressCircle progress={p} />
            {p.closed}/{p.total}
          </span>
        </span>
      </li>
    );
  };

  const renderIssueList = (hideWhenCollapsed: boolean) => (
    <ul className={`${styles.list} ${hideWhenCollapsed && collapsed ? styles.listCollapsed : ""}`}>
      <li
        className={`${styles.item} ${activeFilter === "inbox" ? styles.active : ""}`}
        onClick={() => handleFilterChange("inbox")}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleFilterChange("inbox");
          }
        }}
      >
        <span className={styles.itemLabel}>Inbox</span>
        {inboxCount > 0 && (
          <span className={styles.inboxCount}>{inboxCount}</span>
        )}
      </li>
      <li
        className={`${styles.item} ${activeFilter === null ? styles.active : ""}`}
        onClick={() => handleFilterChange(null)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleFilterChange(null);
          }
        }}
      >
        <span className={styles.itemLabel}>Everything</span>
      </li>
      {activeList.map(renderItem)}
      {completedList.length > 0 && (
        <>
          <li
            className={styles.completedToggle}
            onClick={() => setCompletedExpanded((v) => !v)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                setCompletedExpanded((v) => !v);
              }
            }}
          >
            <span className={styles.completedToggleLabel}>
              {completedExpanded ? "\u25BC" : "\u25B6"} Completed ({completedList.length})
            </span>
          </li>
          {completedExpanded && completedList.map(renderItem)}
        </>
      )}
    </ul>
  );

  return (
    <>
      {/* Desktop sidebar — hidden on mobile via CSS */}
      <div className={`${styles.sidebar} ${collapsed ? styles.collapsed : ""}`}>
        {!collapsed && (
          <div className={styles.header}>
            <span className={styles.title}>Issues</span>
          </div>
        )}
        {renderIssueList(true)}
      </div>

      {/* Mobile slide-out drawer (hamburger button lives in HeterogeneousItemList toolbar) */}
      {drawerOpen && (
        <div
          className={styles.backdrop}
          onClick={onDrawerClose}
        />
      )}
      <div className={`${styles.drawer} ${drawerOpen ? styles.drawerOpen : ""}`}>
        <div className={styles.drawerHeader}>
          <span className={styles.title}>Issues</span>
        </div>
        {renderIssueList(false)}
      </div>
    </>
  );
}
