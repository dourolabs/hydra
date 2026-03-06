import { useMemo } from "react";
import type { IssueSummaryRecord, LabelRecord } from "@metis/api";
import { useLabels } from "../labels/useLabels";
import type { ChildStatus } from "./computeIssueProgress";
import { StatusBoxes } from "./StatusBoxes";
import styles from "./IssueFilterSidebar.module.css";

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
  isActiveMap: Map<string, boolean>,
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

      const assignedToUser = !!(username && issue.issue.assignee === username);

      children.push({
        id: issue.issue_id,
        status,
        hasActiveTask: isActiveMap.get(issue.issue_id) ?? false,
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

interface IssueFilterSidebarProps {
  allIssues: IssueSummaryRecord[];
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
  collapsed: boolean;
  drawerOpen: boolean;
  onDrawerClose: () => void;
  isActiveMap: Map<string, boolean>;
  username: string;
  inboxCount: number;
  myIssuesCount: number;
}

export function IssueFilterSidebar({
  allIssues,
  activeFilter,
  onFilterChange,
  collapsed,
  drawerOpen,
  onDrawerClose,
  isActiveMap,
  username,
  inboxCount,
  myIssuesCount,
}: IssueFilterSidebarProps) {
  /** On mobile, selecting an issue should also close the drawer. */
  const handleFilterChange = (rootId: string | null) => {
    onFilterChange(rootId);
    onDrawerClose();
  };

  const { data: labels } = useLabels();

  const labelProgressList = useMemo(() => {
    if (!labels || labels.length === 0) return [];
    return computeLabelProgress(labels, allIssues, isActiveMap, username);
  }, [labels, allIssues, isActiveMap, username]);

  if (labelProgressList.length === 0) return null;

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
        className={`${styles.item} ${activeFilter === "my-issues" ? styles.active : ""}`}
        onClick={() => handleFilterChange("my-issues")}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleFilterChange("my-issues");
          }
        }}
      >
        <span className={styles.itemLabel}>My Issues</span>
        {myIssuesCount > 0 && (
          <span className={styles.inboxCount}>{myIssuesCount}</span>
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
