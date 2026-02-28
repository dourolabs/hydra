import { useState, useMemo, useCallback } from "react";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";
import { treeHasActiveNode } from "./watchingUtils";
import { descriptionSnippet } from "../../utils/text";
import { getRuntime, formatRelativeTime } from "../../utils/time";
import {
  collectActivityItems,
  sortActivityItems,
  computeSummary,
  stateLabel,
  type ActivityItem,
  type ActivityState,
} from "./activityUtils";
import { WatchlistSidebar } from "./WatchlistSidebar";
import styles from "./WatchlistActivityFeed.module.css";

interface WatchlistActivityFeedProps {
  issues: IssueSummaryRecord[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  username: string;
}

const indicatorClass: Record<ActivityState, string> = {
  active: styles.active,
  "needs-review": styles.needsReview,
  blocked: styles.blocked,
  failed: styles.failed,
  done: styles.done,
  "idle-open": styles.idleOpen,
};

function FeedItemRow({
  item,
  selected,
  onClick,
}: {
  item: ActivityItem;
  selected: boolean;
  onClick: () => void;
}) {
  const desc = descriptionSnippet(item.issue.issue.description, 60);
  const parentSnippet = descriptionSnippet(item.parentDescription, 40);
  const isFaded = item.state === "done";
  const stateClass = indicatorClass[item.state];

  let metaText: string;
  if (item.state === "active" && item.activeJob) {
    metaText = `running for ${getRuntime(item.activeJob.task.start_time, null)}`;
  } else if (item.state === "done") {
    metaText = `completed ${formatRelativeTime(item.issue.timestamp)}`;
  } else if (item.state === "needs-review") {
    const patchCount = item.patchIds.length;
    metaText = patchCount === 1 ? "PR needs review" : `${patchCount} PRs need review`;
  } else if (item.state === "failed") {
    metaText = "failed";
  } else if (item.state === "blocked") {
    metaText = "blocked on dependency";
  } else {
    metaText = formatRelativeTime(item.issue.timestamp);
  }

  const rowClasses = [styles.feedItem];
  if (selected) rowClasses.push(styles.selected);
  if (isFaded) rowClasses.push(styles.faded);

  return (
    <li
      className={rowClasses.join(" ")}
      onClick={onClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
    >
      <span className={`${styles.stateIndicator} ${stateClass}`} />
      <span className={styles.feedItemContent}>
        <span className={styles.feedItemTop}>
          <span className={`${styles.stateLabel} ${stateClass}`}>
            {stateLabel(item.state)}
          </span>
          <span className={styles.feedItemDesc}>{desc}</span>
        </span>
        <span className={styles.feedItemMeta}>
          <span className={styles.parentDesc}>{parentSnippet}</span>
          <span className={styles.separator}>&middot;</span>
          <span className={styles.runtime}>{metaText}</span>
        </span>
      </span>
    </li>
  );
}

export function WatchlistActivityFeed({
  issues,
  jobsByIssue,
  selectedId,
  onSelect,
  username,
}: WatchlistActivityFeedProps) {
  const watchingRoots = useMemo(() => {
    const tree = buildIssueTree(issues);
    return tree
      .filter(
        (root) =>
          !root.hardBlocked &&
          root.issue.issue.creator === username &&
          treeHasActiveNode(root, jobsByIssue),
      )
      .sort(
        (a, b) =>
          new Date(b.issue.creation_time).getTime() -
          new Date(a.issue.creation_time).getTime(),
      );
  }, [issues, jobsByIssue, username]);

  const allItems = useMemo(() => {
    const raw = collectActivityItems(watchingRoots, jobsByIssue);
    return sortActivityItems(raw);
  }, [watchingRoots, jobsByIssue]);

  const summary = useMemo(() => computeSummary(allItems), [allItems]);

  const { filteredItems, activeFilter, handleFilterChange } = useFilterState(
    allItems,
  );

  const handleItemClick = useCallback(
    (issueId: string) => {
      onSelect(issueId);
    },
    [onSelect],
  );

  if (watchingRoots.length === 0) {
    return <p className={styles.empty}>No issues being watched.</p>;
  }

  // Group items by section
  const activeItems = filteredItems.filter((i) => i.state === "active");
  const attentionItems = filteredItems.filter(
    (i) =>
      i.state === "needs-review" ||
      i.state === "blocked" ||
      i.state === "failed",
  );
  const completedItems = filteredItems.filter((i) => i.state === "done");
  const openItems = filteredItems.filter((i) => i.state === "idle-open");

  return (
    <div className={styles.container}>
      <div className={styles.summaryBar}>
        <span className={styles.summaryItem}>
          <span className={`${styles.summaryDot} ${styles.active}`} />
          {summary.activeCount} active
        </span>
        <span className={styles.summaryItem}>
          <span className={`${styles.summaryDot} ${styles.attention}`} />
          {summary.attentionCount} need attention
        </span>
        <span className={styles.summaryItem}>
          <span className={`${styles.summaryDot} ${styles.done}`} />
          {summary.doneCount}/{summary.totalCount} shipped
        </span>
      </div>
      <div className={styles.splitLayout}>
        <div className={styles.feedColumn}>
          <ul className={styles.feed}>
            {activeItems.length > 0 && (
              <>
                <li className={styles.sectionHeader}>Active</li>
                {activeItems.map((item) => (
                  <FeedItemRow
                    key={item.issueId}
                    item={item}
                    selected={item.issueId === selectedId}
                    onClick={() => handleItemClick(item.issueId)}
                  />
                ))}
              </>
            )}
            {attentionItems.length > 0 && (
              <>
                <li className={styles.sectionHeader}>Needs Attention</li>
                {attentionItems.map((item) => (
                  <FeedItemRow
                    key={item.issueId}
                    item={item}
                    selected={item.issueId === selectedId}
                    onClick={() => handleItemClick(item.issueId)}
                  />
                ))}
              </>
            )}
            {openItems.length > 0 && (
              <>
                <li className={styles.sectionHeader}>Open</li>
                {openItems.map((item) => (
                  <FeedItemRow
                    key={item.issueId}
                    item={item}
                    selected={item.issueId === selectedId}
                    onClick={() => handleItemClick(item.issueId)}
                  />
                ))}
              </>
            )}
            {completedItems.length > 0 && (
              <>
                <li className={styles.sectionHeader}>Recently Completed</li>
                {completedItems.map((item) => (
                  <FeedItemRow
                    key={item.issueId}
                    item={item}
                    selected={item.issueId === selectedId}
                    onClick={() => handleItemClick(item.issueId)}
                  />
                ))}
              </>
            )}
            {filteredItems.length === 0 && (
              <li className={styles.empty}>
                {activeFilter
                  ? "No activity for this issue."
                  : "No activity yet."}
              </li>
            )}
          </ul>
        </div>
        <div className={styles.sidebarColumn}>
          <WatchlistSidebar
            roots={watchingRoots}
            jobsByIssue={jobsByIssue}
            activeFilter={activeFilter}
            onFilterChange={handleFilterChange}
          />
        </div>
      </div>
    </div>
  );
}

/**
 * Hook to manage sidebar filter state.
 * Clicking a sidebar issue filters the feed; clicking again clears the filter.
 */
function useFilterState(allItems: ActivityItem[]) {
  const [activeFilter, setActiveFilter] = useState<string | null>(null);

  const filteredItems = useMemo(() => {
    if (!activeFilter) return allItems;
    return allItems.filter((item) => item.parentIssueId === activeFilter);
  }, [allItems, activeFilter]);

  const handleFilterChange = useCallback(
    (rootId: string | null) => {
      setActiveFilter((prev) => (prev === rootId ? null : rootId));
    },
    [],
  );

  return { filteredItems, activeFilter, handleFilterChange };
}
