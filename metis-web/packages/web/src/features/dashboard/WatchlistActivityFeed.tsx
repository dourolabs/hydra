import { useState, useMemo, useCallback } from "react";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";
import { descriptionSnippet } from "../../utils/text";
import { getRuntime, formatRelativeTime } from "../../utils/time";
import {
  collectActivityItems,
  sortActivityItems,
  computeSummary,
  sectionLabel,
  type ActivityItem,
  type ActivitySection,
} from "./activityUtils";
import { WatchlistSidebar, WatchlistMobilePills } from "./WatchlistSidebar";
import styles from "./WatchlistActivityFeed.module.css";

interface WatchlistActivityFeedProps {
  issues: IssueSummaryRecord[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  selectedId: string | null;
  onSelect: (issueId: string) => void;
  username: string;
}

const indicatorClass: Record<ActivitySection, string> = {
  active: styles.active,
  "needs-attention": styles.needsAttention,
  upcoming: styles.upcoming,
  "recently-completed": styles.completed,
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
  const isFaded = item.section === "recently-completed";
  const stateClass = indicatorClass[item.section];

  let metaText: string;
  if (item.section === "active" && item.activeJob) {
    metaText = `running for ${getRuntime(item.activeJob.task.start_time, null)}`;
  } else if (item.section === "recently-completed") {
    metaText = `completed ${formatRelativeTime(item.issue.timestamp)}`;
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
            {sectionLabel(item.section)}
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

function renderSection(
  items: ActivityItem[],
  label: string,
  selectedId: string | null,
  onItemClick: (issueId: string) => void,
) {
  if (items.length === 0) return null;
  return (
    <>
      <li className={styles.sectionHeader}>{label}</li>
      {items.map((item) => (
        <FeedItemRow
          key={item.issueId}
          item={item}
          selected={item.issueId === selectedId}
          onClick={() => onItemClick(item.issueId)}
        />
      ))}
    </>
  );
}

export function WatchlistActivityFeed({
  issues,
  jobsByIssue,
  selectedId,
  onSelect,
  username,
}: WatchlistActivityFeedProps) {
  const [filterRootId, setFilterRootId] = useState<string | null>(null);

  const roots = useMemo(() => {
    const tree = buildIssueTree(issues);
    return tree
      .filter(
        (root) =>
          !root.hardBlocked && root.issue.issue.creator === username,
      )
      .sort(
        (a, b) =>
          new Date(b.issue.creation_time).getTime() -
          new Date(a.issue.creation_time).getTime(),
      );
  }, [issues, username]);

  const allItems = useMemo(() => {
    const raw = collectActivityItems(roots, jobsByIssue, username);
    return sortActivityItems(raw);
  }, [roots, jobsByIssue, username]);

  const displayItems = useMemo(() => {
    if (!filterRootId) return allItems;
    return allItems.filter((item) => item.parentIssueId === filterRootId);
  }, [allItems, filterRootId]);

  const summary = useMemo(() => computeSummary(displayItems), [displayItems]);

  const handleItemClick = useCallback(
    (issueId: string) => {
      onSelect(issueId);
    },
    [onSelect],
  );

  const activeItems = displayItems.filter((i) => i.section === "active");
  const attentionItems = displayItems.filter(
    (i) => i.section === "needs-attention",
  );
  const upcomingItems = displayItems.filter((i) => i.section === "upcoming");
  const completedItems = displayItems.filter(
    (i) => i.section === "recently-completed",
  );

  const sidebarProps = {
    roots,
    activeFilter: filterRootId,
    onFilterChange: setFilterRootId,
  };

  return (
    <div className={styles.container}>
      <div className={styles.feedColumn}>
        <WatchlistMobilePills {...sidebarProps} />
        <div className={styles.summaryBar}>
          <span className={styles.summaryItem}>
            <span className={`${styles.summaryDot} ${styles.active}`} />
            {summary.activeCount} active
          </span>
          <span className={styles.summaryItem}>
            <span className={`${styles.summaryDot} ${styles.attention}`} />
            {summary.needsAttentionCount} need attention
          </span>
          <span className={styles.summaryItem}>
            <span className={`${styles.summaryDot} ${styles.done}`} />
            {summary.completedCount}/{summary.totalCount} completed
          </span>
        </div>
        <ul className={styles.feed}>
          {renderSection(activeItems, "Active", selectedId, handleItemClick)}
          {renderSection(
            attentionItems,
            "Needs Attention",
            selectedId,
            handleItemClick,
          )}
          {renderSection(
            upcomingItems,
            "Upcoming",
            selectedId,
            handleItemClick,
          )}
          {renderSection(
            completedItems,
            "Recently Completed",
            selectedId,
            handleItemClick,
          )}
          {displayItems.length === 0 && (
            <li className={styles.empty}>No activity yet.</li>
          )}
        </ul>
      </div>
      <WatchlistSidebar {...sidebarProps} />
    </div>
  );
}
