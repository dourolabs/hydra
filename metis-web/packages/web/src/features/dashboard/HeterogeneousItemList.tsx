import { useMemo, useCallback } from "react";
import type { JobSummaryRecord } from "@metis/api";
import type { WorkItem } from "./useTransitiveWorkItems";
import { useItemNotifications } from "./useItemNotifications";
import { ItemRow } from "./ItemRow";
import styles from "./HeterogeneousItemList.module.css";

interface HeterogeneousItemListProps {
  items: WorkItem[];
  jobsByIssue: Map<string, JobSummaryRecord[]>;
  isLoading: boolean;
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  onToggleDrawer: () => void;
}

/** Active (non-terminal) statuses for segmentation. */
function isActiveItem(item: WorkItem): boolean {
  return !item.isTerminal;
}

function sortByLastUpdated(a: WorkItem, b: WorkItem): number {
  return new Date(b.lastUpdated).getTime() - new Date(a.lastUpdated).getTime();
}

export function HeterogeneousItemList({
  items,
  jobsByIssue,
  isLoading,
  sidebarCollapsed,
  onToggleSidebar,
  onToggleDrawer,
}: HeterogeneousItemListProps) {
  const { getItemNotification, markItemRead, notificationMap } =
    useItemNotifications(items);

  // Sort: unread items first, then by timestamp (descending)
  const sortWithUnread = useCallback(
    (a: WorkItem, b: WorkItem): number => {
      const aUnread = notificationMap.has(`${a.kind}:${a.id}`) ? 1 : 0;
      const bUnread = notificationMap.has(`${b.kind}:${b.id}`) ? 1 : 0;
      if (aUnread !== bUnread) return bUnread - aUnread;
      return sortByLastUpdated(a, b);
    },
    [notificationMap],
  );

  const activeItems = useMemo(
    () => items.filter(isActiveItem).sort(sortWithUnread),
    [items, sortWithUnread],
  );

  const completeItems = useMemo(
    () => items.filter((i) => !isActiveItem(i)).sort(sortWithUnread),
    [items, sortWithUnread],
  );

  return (
    <div className={styles.container}>
      <div className={styles.toolbar}>
        <button
          type="button"
          className={styles.sidebarToggle}
          onClick={onToggleSidebar}
          aria-label={
            sidebarCollapsed
              ? "Expand filter sidebar"
              : "Collapse filter sidebar"
          }
        >
          {sidebarCollapsed ? "\u25B6" : "\u25C0"}
        </button>
        <button
          type="button"
          className={styles.drawerToggle}
          onClick={onToggleDrawer}
          aria-label="Open issue menu"
        >
          <svg
            viewBox="0 0 20 20"
            fill="currentColor"
            width="16"
            height="16"
          >
            <path
              fillRule="evenodd"
              d="M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z"
              clipRule="evenodd"
            />
          </svg>
        </button>
        <span className={styles.toolbarSummary}>
          {activeItems.length} active
          {completeItems.length > 0 && (
            <span className={styles.toolbarDivider}>&middot;</span>
          )}
          {completeItems.length > 0 && `${completeItems.length} complete`}
        </span>
      </div>

      <div className={styles.listScroll}>
        {isLoading && items.length === 0 && (
          <div className={styles.empty}>Loading items&hellip;</div>
        )}

        {!isLoading && items.length === 0 && (
          <div className={styles.empty}>No items yet.</div>
        )}

        {activeItems.length > 0 && (
          <>
            <div className={styles.sectionHeader}>
              Active ({activeItems.length})
            </div>
            <ul className={styles.list}>
              {activeItems.map((item) => (
                <ItemRow
                  key={`${item.kind}-${item.id}`}
                  item={item}
                  jobs={
                    item.kind === "issue"
                      ? jobsByIssue.get(item.id)
                      : undefined
                  }
                  notification={getItemNotification(item)}
                  onMarkRead={markItemRead}
                />
              ))}
            </ul>
          </>
        )}

        {completeItems.length > 0 && (
          <>
            <div className={styles.sectionHeader}>
              Complete ({completeItems.length})
            </div>
            <ul className={styles.list}>
              {completeItems.map((item) => (
                <ItemRow
                  key={`${item.kind}-${item.id}`}
                  item={item}
                  jobs={
                    item.kind === "issue"
                      ? jobsByIssue.get(item.id)
                      : undefined
                  }
                  notification={getItemNotification(item)}
                  onMarkRead={markItemRead}
                />
              ))}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}
