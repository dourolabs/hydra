import { useMemo } from "react";
import type { JobSummaryRecord } from "@metis/api";
import type { ChildStatus } from "./computeIssueProgress";
import type { WorkItem } from "./useTransitiveWorkItems";
import { topologicalSortWorkItems } from "../issues/topologicalSort";
import { ItemRow } from "./ItemRow";
import { SearchBox } from "../../components/SearchBox/SearchBox";
import styles from "./HeterogeneousItemList.module.css";

interface HeterogeneousItemListProps {
  items: WorkItem[];
  sessionsByIssue: Map<string, JobSummaryRecord[]>;
  childStatusMap: Map<string, ChildStatus[]>;
  isActiveMap: Map<string, boolean>;
  isLoading: boolean;
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  onToggleDrawer: () => void;
  filterRootId: string | null;
  searchValue: string;
  onSearchChange: (value: string) => void;
  inboxLabelId?: string;
}

/** Artifacts are patches and documents regardless of terminal status. */
function isArtifact(item: WorkItem): boolean {
  return item.kind === "patch" || item.kind === "document";
}

/** Active items are non-terminal issues (excludes artifacts). */
function isActiveItem(item: WorkItem): boolean {
  return item.kind === "issue" && !item.isTerminal;
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
  sidebarCollapsed,
  onToggleSidebar,
  onToggleDrawer,
  filterRootId,
  searchValue,
  onSearchChange,
  inboxLabelId,
}: HeterogeneousItemListProps) {
  const activeItems = useMemo(
    () => topologicalSortWorkItems(items.filter(isActiveItem)),
    [items],
  );

  const artifactItems = useMemo(
    () => items.filter(isArtifact).sort(sortByLastUpdated),
    [items],
  );

  const completeItems = useMemo(
    () =>
      items
        .filter((i) => i.kind === "issue" && i.isTerminal)
        .sort(sortByLastUpdated),
    [items],
  );

  const hamburgerButton = (
    <button
      type="button"
      className={styles.drawerToggle}
      onClick={(e) => {
        e.stopPropagation();
        onToggleDrawer();
      }}
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
        <SearchBox
          value={searchValue}
          onChange={onSearchChange}
          placeholder="Search issues..."
          leftElement={hamburgerButton}
        />
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
                  filterRootId={filterRootId}
                  inboxLabelId={inboxLabelId}

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
                  filterRootId={filterRootId}
                  inboxLabelId={inboxLabelId}

                />
              ))}
            </ul>
          </>
        )}

        {artifactItems.length > 0 && (
          <>
            <div className={styles.sectionHeader}>
              Artifacts ({artifactItems.length})
            </div>
            <ul className={styles.list}>
              {artifactItems.map((item) => (
                <ItemRow
                  key={`${item.kind}-${item.id}`}
                  item={item}
                  sessions={undefined}
                  filterRootId={filterRootId}

                />
              ))}

            </ul>
          </>
        )}
      </div>
    </div>
  );
}
