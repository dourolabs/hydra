import { useMemo } from "react";
import type { IssueTreeNode } from "../issues/useIssues";
import { descriptionSnippet } from "../../utils/text";
import { computeIssueProgress } from "./activityUtils";
import styles from "./IssueFilterDropdown.module.css";

interface IssueFilterProps {
  roots: IssueTreeNode[];
  activeFilter: string | null;
  onFilterChange: (rootId: string | null) => void;
}

/**
 * Dropdown filter for selecting which root issue to view in the activity feed.
 * Replaces the sidebar column — the feed is now full-width.
 */
export function IssueFilterDropdown({
  roots,
  activeFilter,
  onFilterChange,
}: IssueFilterProps) {
  const progressList = useMemo(() => computeIssueProgress(roots), [roots]);

  if (progressList.length === 0) return null;

  return (
    <div className={styles.filterWrapper}>
      <select
        className={styles.filterSelect}
        value={activeFilter ?? ""}
        onChange={(e) => {
          const val = e.target.value;
          onFilterChange(val === "" ? null : val);
        }}
      >
        <option value="">All issues</option>
        {progressList.map((p) => {
          const label = descriptionSnippet(
            p.rootIssue.issue.description,
            40,
          );
          const stats = `${p.closed}/${p.total} done`;
          return (
            <option key={p.rootId} value={p.rootId}>
              {label} ({stats})
            </option>
          );
        })}
      </select>
      <span className={styles.filterChevron}>&#9662;</span>
    </div>
  );
}
