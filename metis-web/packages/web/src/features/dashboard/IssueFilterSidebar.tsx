import { useMemo, useState } from "react";
import type { IssueSummaryRecord, JobSummaryRecord, LabelRecord } from "@metis/api";
import type { IssueTreeNode } from "../issues/useIssues";
import { useLabels } from "../labels/useLabels";
import { descriptionSnippet } from "../../utils/text";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { computeIssueProgress, type ChildStatus, type IssueProgress } from "./computeIssueProgress";
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
  inboxCount: number;
}

function getBoxClass(child: ChildStatus): string {
  // Priority: active task > assigned to user (open) > base status
  if (child.hasActiveTask) return styles.statusBoxActive;
  if (child.assignedToUser && child.status === "open") return styles.statusBoxAttention;
  if (child.status === "closed") return styles.statusBoxClosed;
  if (child.status === "in-progress") return styles.statusBoxInProgress;
  if (child.status === "failed") return styles.statusBoxFailed;
  return styles.statusBoxOpen;
}

/** Row of small colored squares — one per child issue, color-coded by status. */
function StatusBoxes({ children }: { children: ChildStatus[] }) {
  if (children.length === 0) return null;

  return (
    <span className={styles.statusBoxes}>
      {children.map((child) => (
        <span key={child.id} className={`${styles.statusBox} ${getBoxClass(child)}`} />
      ))}
    </span>
  );
}

/** Label filter prefix used in activeFilter to distinguish label filters from issue filters. */
export const LABEL_FILTER_PREFIX = "label:";

interface LabelProgress {
  labelId: string;
  name: string;
  color: string;
  closed: number;
  total: number;
  children: ChildStatus[];
}

function computeLabelProgress(
  labels: LabelRecord[],
  allIssues: IssueSummaryRecord[],
  jobsByIssue: Map<string, JobSummaryRecord[]>,
  username: string,
): LabelProgress[] {
  return labels.map((label) => {
    const labelIssues = allIssues.filter((issue) =>
      issue.issue.labels?.some((l: { label_id: string }) => l.label_id === label.label_id),
    );

    let closed = 0;
    const children: ChildStatus[] = [];

    for (const issue of labelIssues) {
      const status = issue.issue.status;
      if (status === "closed") closed++;

      const jobs = jobsByIssue.get(issue.issue_id) ?? [];
      const hasActiveTask = jobs.some(
        (j) => j.task.status === "running" || j.task.status === "pending",
      );
      const assignedToUser = !!(username && issue.issue.assignee === username);

      children.push({
        id: issue.issue_id,
        status,
        hasActiveTask,
        assignedToUser,
      });
    }

    return {
      labelId: label.label_id,
      name: label.name,
      color: label.color,
      closed,
      total: labelIssues.length,
      children,
    };
  });
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
  inboxCount,
}: IssueFilterSidebarProps) {
  /** On mobile, selecting an issue should also close the drawer. */
  const handleFilterChange = (rootId: string | null) => {
    onFilterChange(rootId);
    onDrawerClose();
  };

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

  const { data: labels } = useLabels();

  const labelProgressList = useMemo(() => {
    if (!labels || labels.length === 0) return [];
    return computeLabelProgress(labels, allIssues, jobsByIssue, username);
  }, [labels, allIssues, jobsByIssue, username]);

  if (progressList.length === 0 && labelProgressList.length === 0) return null;

  const renderItem = (p: IssueProgress) => {
    const label = p.rootIssue.issue.title || descriptionSnippet(p.rootIssue.issue.description, 80);
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
        <span className={styles.itemLeft}>
          <span className={styles.itemLabel}>{label}</span>
          <span className={styles.itemStats}>
            <StatusBoxes children={p.children} />
            {p.closed}/{p.total}
          </span>
        </span>
        {p.needsAttentionCount > 0 && (
          <span className={styles.needsAttentionChip}>
            {p.needsAttentionCount}
          </span>
        )}
      </li>
    );
  };

  const renderLabelItem = (lp: LabelProgress) => {
    const filterId = `${LABEL_FILTER_PREFIX}${lp.labelId}`;
    const isActive = activeFilter === filterId;
    return (
      <li
        key={lp.labelId}
        className={`${styles.item} ${isActive ? styles.active : ""}`}
        onClick={() => handleFilterChange(isActive ? null : filterId)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleFilterChange(isActive ? null : filterId);
          }
        }}
      >
        <span className={styles.itemLeft}>
          <span className={styles.itemLabel}>
            <span
              className={styles.labelDot}
              style={{ background: lp.color }}
            />
            {lp.name}
          </span>
          <span className={styles.itemStats}>
            <StatusBoxes children={lp.children} />
            {lp.closed}/{lp.total}
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
      {labelProgressList.length > 0 && (
        <>
          <li className={styles.labelSectionHeader}>Labels</li>
          {labelProgressList.map(renderLabelItem)}
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
