import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";
import { Icons } from "@hydra/ui";
import type { IssueNeighborhood } from "../flowPill";
import type { IssueFilters } from "../usePaginatedIssues";
import { FilterBar, type Filter, type FilterDefinitions } from "../../filters";
import { IssuesTable } from "./IssuesTable";
import { IssuesBoard } from "./IssuesBoard";
import styles from "./IssuesView.module.css";

export type IssuesLayout = "table" | "board";

interface IssuesViewProps {
  layout: IssuesLayout;
  onLayoutChange: (layout: IssuesLayout) => void;
  // Table-only data (board owns its own fetches via baseFilters).
  issues: IssueSummaryRecord[];
  neighborhoodMap: Map<string, IssueNeighborhood>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
  // Board layout still needs these to feed its per-column queries; table
  // layout drives them via the FilterBar / search input.
  baseFilters: IssueFilters;
  filterRootId: string | null;
  eyebrow: string;
  title: string;
  // Table-mode FilterBar state. The page owns this and persists it to URL;
  // IssuesView is a dumb consumer that renders the bar + search input.
  filters: Filter[];
  setFilters: (next: Filter[]) => void;
  definitions: FilterDefinitions<IssueSummaryRecord>;
  filteredCount: number;
  totalCount: number;
  searchValue: string;
  onSearchChange: (value: string) => void;
  // Passed through to <FilterBar onMenuOpenChange>. The page uses this to
  // lazy-load relation-picker option lists only when the menu opens.
  onFilterMenuOpenChange?: (open: boolean) => void;
}

export function IssuesView({
  layout,
  onLayoutChange,
  issues,
  neighborhoodMap,
  sessionsByIssue,
  isLoading,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  baseFilters,
  filterRootId,
  eyebrow,
  title,
  filters,
  setFilters,
  definitions,
  filteredCount,
  totalCount,
  searchValue,
  onSearchChange,
  onFilterMenuOpenChange,
}: IssuesViewProps) {
  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>{eyebrow}</span>
          <h1 className={styles.pageTitle}>{title}</h1>
        </div>
        <span className={styles.headSpacer} />
        <div className={styles.headRight}>
          <div className={styles.segmented} role="tablist" aria-label="Layout">
            <button
              type="button"
              role="tab"
              aria-selected={layout === "table"}
              className={layout === "table" ? styles.segmentedActive : undefined}
              onClick={() => onLayoutChange("table")}
              data-testid="issues-layout-table"
            >
              <Icons.IconMenu size={14} />
              Table
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={layout === "board"}
              className={layout === "board" ? styles.segmentedActive : undefined}
              onClick={() => onLayoutChange("board")}
              data-testid="issues-layout-board"
            >
              <Icons.IconDot size={14} />
              Board
            </button>
          </div>
        </div>
      </div>

      {layout === "table" && (
        <div className={styles.toolbar}>
          <div className={styles.searchBox}>
            <span className={styles.searchIcon}>
              <Icons.IconSearch size={14} />
            </span>
            <input
              type="text"
              placeholder="Search issues…"
              value={searchValue}
              onChange={(e) => onSearchChange(e.target.value)}
              aria-label="Search issues"
              data-testid="issues-search"
            />
          </div>
          <FilterBar
            filters={filters}
            setFilters={setFilters}
            definitions={definitions}
            count={filteredCount}
            total={totalCount}
            onMenuOpenChange={onFilterMenuOpenChange}
          />
        </div>
      )}

      <div className={styles.body}>
        {layout === "table" && (
          <>
            {isLoading && issues.length === 0 && (
              <div className={styles.empty}>Loading issues…</div>
            )}

            {!isLoading && issues.length === 0 && (
              <div className={styles.empty}>No issues match the current filters.</div>
            )}

            {issues.length > 0 && (
              <IssuesTable
                issues={issues}
                neighborhoodMap={neighborhoodMap}
                sessionsByIssue={sessionsByIssue}
                filterRootId={filterRootId}
              />
            )}

            {hasNextPage && (
              <div className={styles.loadMore}>
                <button
                  type="button"
                  className={styles.loadMoreButton}
                  onClick={onLoadMore}
                  disabled={isFetchingNextPage}
                >
                  {isFetchingNextPage ? "Loading…" : "Load more"}
                </button>
              </div>
            )}
          </>
        )}

        {layout === "board" && (
          <IssuesBoard
            baseFilters={baseFilters}
            filterRootId={filterRootId}
          />
        )}
      </div>
    </div>
  );
}
