import { useMemo, type ReactNode } from "react";
import type { SessionSummaryRecord } from "@hydra/api";
import type { ChildStatus } from "./computeIssueProgress";
import type { WorkItem } from "./workItemTypes";
import { ItemRow } from "./ItemRow";
import { SearchBox } from "../../components/SearchBox/SearchBox";
import styles from "./HeterogeneousItemList.module.css";

interface HeterogeneousItemListProps {
  items: WorkItem[];
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  childStatusMap: Map<string, ChildStatus[]>;
  isActiveMap: Map<string, boolean>;
  isLoading: boolean;
  treeLoading?: boolean;
  filterRootId: string | null;
  searchValue: string;
  onSearchChange: (value: string) => void;
  inboxLabelId?: string;
  hasNextPage?: boolean;
  isFetchingNextPage?: boolean;
  onLoadMore?: () => void;
  filterBar?: ReactNode;
}

function sortByLastUpdated(a: WorkItem, b: WorkItem): number {
  return new Date(b.lastUpdated).getTime() - new Date(a.lastUpdated).getTime();
}

export function HeterogeneousItemList({
  items,
  sessionsByIssue,
  childStatusMap,
  isActiveMap,
  isLoading,
  treeLoading,
  filterRootId,
  searchValue,
  onSearchChange,
  inboxLabelId,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  filterBar,
}: HeterogeneousItemListProps) {
  const sortedItems = useMemo(
    () => [...items].sort(sortByLastUpdated),
    [items],
  );

  return (
    <div className={styles.container}>
      <div className={styles.toolbar}>
        <SearchBox
          value={searchValue}
          onChange={onSearchChange}
          placeholder="Search..."
        />
      </div>

      {filterBar}

      <div className={styles.listScroll}>
        {isLoading && items.length === 0 && (
          <div className={styles.empty}>Loading items&hellip;</div>
        )}

        {!isLoading && items.length === 0 && (
          <div className={styles.empty}>No items yet.</div>
        )}

        {sortedItems.length > 0 && (
          <ul className={styles.list}>
            {sortedItems.map((item) => (
              <ItemRow
                key={`${item.kind}-${item.id}`}
                item={item}
                sessions={
                  item.kind === "issue"
                    ? sessionsByIssue.get(item.id)
                    : undefined
                }
                childStatuses={
                  item.kind === "issue"
                    ? childStatusMap.get(item.id)
                    : undefined
                }
                isActive={item.kind === "issue" ? (isActiveMap.get(item.id) ?? false) : false}
                treeLoading={treeLoading}
                filterRootId={filterRootId}
                inboxLabelId={inboxLabelId}
              />
            ))}
          </ul>
        )}

        {hasNextPage && (
          <div className={styles.loadMore}>
            <button
              type="button"
              className={styles.loadMoreButton}
              onClick={onLoadMore}
              disabled={isFetchingNextPage}
            >
              {isFetchingNextPage ? "Loading..." : "Load more"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
