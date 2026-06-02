import { useMemo, useState } from "react";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";
import { Icons } from "@hydra/ui";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import type { IssueFilters } from "../usePaginatedIssues";
import { FilterBar, applyFilters, type Filter } from "../../filters";
import { useIssueFilters } from "../issueFilters";
import { IssuesTable } from "./IssuesTable";
import { IssuesBoard } from "./IssuesBoard";
import styles from "./IssuesView.module.css";

export type IssuesLayout = "table" | "board";

interface IssuesViewProps {
  layout: IssuesLayout;
  onLayoutChange: (layout: IssuesLayout) => void;
  // Table-only data (board owns its own fetches via baseFilters/username).
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
  // Board layout still needs these to feed its per-column queries; table
  // layout now narrows purely client-side via the new <FilterBar>.
  baseFilters: IssueFilters;
  username: string;
  filterRootId: string | null;
  eyebrow: string;
  title: string;
}

export function IssuesView({
  layout,
  onLayoutChange,
  issues,
  childStatusMap,
  sessionsByIssue,
  isLoading,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  baseFilters,
  username,
  filterRootId,
  eyebrow,
  title,
}: IssuesViewProps) {
  const definitions = useIssueFilters({ loadedIssues: issues });
  const [filters, setFilters] = useState<Filter[]>([]);

  const filtered = useMemo(
    () => applyFilters(issues, filters, definitions),
    [issues, filters, definitions],
  );

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
          <FilterBar
            filters={filters}
            setFilters={setFilters}
            definitions={definitions}
            count={filtered.length}
            total={issues.length}
          />
        </div>
      )}

      <div className={styles.body}>
        {layout === "table" && (
          <>
            {isLoading && filtered.length === 0 && (
              <div className={styles.empty}>Loading issues…</div>
            )}

            {!isLoading && filtered.length === 0 && (
              <div className={styles.empty}>No issues match the current filters.</div>
            )}

            {filtered.length > 0 && (
              <IssuesTable
                issues={filtered}
                childStatusMap={childStatusMap}
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
            username={username}
            filterRootId={filterRootId}
          />
        )}
      </div>
    </div>
  );
}
