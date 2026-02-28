import { useMemo } from "react";
import type { IssueTreeNode } from "../issues/useIssues";
import { descriptionSnippet } from "../../utils/text";
import { computeIssueProgress } from "./activityUtils";
import { writeCollapsed } from "./sidebarStorage";
import styles from "./IssueFilterSidebar.module.css";

interface IssueFilterSidebarProps {
  roots: IssueTreeNode[];
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
  collapsed: boolean;
  onToggleCollapsed: (collapsed: boolean) => void;
}

export function IssueFilterSidebar({
  roots,
  activeFilter,
  onFilterChange,
  collapsed,
  onToggleCollapsed,
}: IssueFilterSidebarProps) {
  const progressList = useMemo(() => computeIssueProgress(roots), [roots]);

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
      {!collapsed && (
        <ul className={styles.list}>
          <li
            className={`${styles.item} ${activeFilter === null ? styles.active : ""}`}
            onClick={() => onFilterChange(null)}
            role="button"
            tabIndex={0}
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
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onFilterChange(p.rootId);
                  }
                }}
              >
                <span className={styles.itemLabel}>{label}</span>
                <span className={styles.itemStats}>
                  {p.closed}/{p.total}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
