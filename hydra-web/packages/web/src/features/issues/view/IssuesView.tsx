import { useEffect, useState } from "react";
import type {
  IssueStatus,
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { Icons, Kbd } from "@hydra/ui";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import { IssuesTable } from "./IssuesTable";
import { IssuesBoard } from "./IssuesBoard";
import styles from "./IssuesView.module.css";

const LAYOUT_STORAGE_KEY = "hydra:issues:layout";

export type IssuesLayout = "table" | "board";

interface IssueStatusFilter {
  key: "all" | IssueStatus;
  label: string;
}

const STATUS_FILTERS: IssueStatusFilter[] = [
  { key: "all", label: "All" },
  { key: "open", label: "Open" },
  { key: "in-progress", label: "In progress" },
  { key: "failed", label: "Failed" },
  { key: "closed", label: "Closed" },
  { key: "dropped", label: "Dropped" },
];

interface IssuesViewProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
  filterRootId: string | null;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
  searchValue: string;
  onSearchChange: (value: string) => void;
  // Server-side status filter (optional). When set, this becomes the active chip
  // and the chip click should propagate up so the backing query can be updated.
  selectedStatus: IssueStatus | null;
  onStatusChange: (status: IssueStatus | null) => void;
  // Eyebrow text — e.g. "WORK · 42 ISSUES" or "ASSIGNED · 8 ISSUES"
  eyebrow: string;
  title: string;
}

function readLayout(): IssuesLayout {
  if (typeof window === "undefined") return "table";
  try {
    const v = window.localStorage.getItem(LAYOUT_STORAGE_KEY);
    if (v === "board" || v === "table") return v;
  } catch {
    /* ignore */
  }
  return "table";
}

function writeLayout(layout: IssuesLayout): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LAYOUT_STORAGE_KEY, layout);
  } catch {
    /* ignore */
  }
}

export function IssuesView({
  issues,
  childStatusMap,
  sessionsByIssue,
  isLoading,
  filterRootId,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  searchValue,
  onSearchChange,
  selectedStatus,
  onStatusChange,
  eyebrow,
  title,
}: IssuesViewProps) {
  const [layout, setLayout] = useState<IssuesLayout>(readLayout);

  useEffect(() => {
    writeLayout(layout);
  }, [layout]);

  const handleStatusChip = (key: IssueStatusFilter["key"]) => {
    onStatusChange(key === "all" ? null : key);
  };

  const activeKey: IssueStatusFilter["key"] = selectedStatus ?? "all";

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
              onClick={() => setLayout("table")}
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
              onClick={() => setLayout("board")}
              data-testid="issues-layout-board"
            >
              <Icons.IconDot size={14} />
              Board
            </button>
          </div>
        </div>
      </div>

      <div className={styles.toolbar}>
        {STATUS_FILTERS.map((filter) => (
          <button
            key={filter.key}
            type="button"
            className={`${styles.chipFilter}${activeKey === filter.key ? ` ${styles.chipFilterActive}` : ""}`}
            onClick={() => handleStatusChip(filter.key)}
            data-testid={`issues-filter-${filter.key}`}
          >
            <span>{filter.label}</span>
          </button>
        ))}
        <span className={styles.toolbarSpacer} />
        <div className={styles.searchBox}>
          <Icons.IconSearch className={styles.searchIcon} size={14} />
          <input
            type="text"
            placeholder="Search issues…"
            value={searchValue}
            onChange={(e) => onSearchChange(e.target.value)}
            aria-label="Search issues"
            data-testid="issues-search"
          />
          <Kbd>/</Kbd>
        </div>
      </div>

      <div className={styles.body}>
        {isLoading && issues.length === 0 && (
          <div className={styles.empty}>Loading issues…</div>
        )}

        {!isLoading && issues.length === 0 && (
          <div className={styles.empty}>No issues match the current filters.</div>
        )}

        {issues.length > 0 && layout === "table" && (
          <IssuesTable
            issues={issues}
            childStatusMap={childStatusMap}
            sessionsByIssue={sessionsByIssue}
            filterRootId={filterRootId}
          />
        )}

        {issues.length > 0 && layout === "board" && (
          <IssuesBoard
            issues={issues}
            childStatusMap={childStatusMap}
            filterRootId={filterRootId}
          />
        )}

        {hasNextPage && layout === "table" && (
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
      </div>
    </div>
  );
}
