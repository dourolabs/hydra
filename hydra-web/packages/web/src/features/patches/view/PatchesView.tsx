import { useNavigate } from "react-router-dom";
import { Avatar, Badge, Icons, Kbd } from "@hydra/ui";
import type { PatchSummaryRecord } from "@hydra/api";
import { normalizePatchStatus } from "../../../utils/badgeStatus";
import { useMediaQuery } from "../../../hooks/useMediaQuery";
import { AgoTime } from "../../../components/Runtime/Runtime";
import { PatchRailRow } from "../../related/RailRow";
import { PatchRepoLink } from "../PatchRepoLink";
import {
  FilterBar,
  type Filter,
  type FilterDefinitions,
} from "../../filters";
import styles from "./PatchesView.module.css";

const MOBILE_QUERY = "(max-width: 768px)";

interface PatchesViewProps {
  patches: PatchSummaryRecord[];
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
  eyebrow: string;
  // FilterBar state. The page owns this and persists it to URL; PatchesView
  // is a dumb consumer that renders the bar + search input.
  filters: Filter[];
  setFilters: (next: Filter[]) => void;
  definitions: FilterDefinitions<PatchSummaryRecord>;
  filteredCount: number;
  totalCount: number;
  searchValue: string;
  onSearchChange: (value: string) => void;
  // Passed through to <FilterBar onMenuOpenChange>. The page uses this to
  // lazy-load relation-picker option lists only when the menu opens.
  onFilterMenuOpenChange?: (open: boolean) => void;
}

export function PatchesView({
  patches,
  isLoading,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  eyebrow,
  filters,
  setFilters,
  definitions,
  filteredCount,
  totalCount,
  searchValue,
  onSearchChange,
  onFilterMenuOpenChange,
}: PatchesViewProps) {
  const navigate = useNavigate();
  const isMobile = useMediaQuery(MOBILE_QUERY);

  const handleRowClick = (id: string) => {
    navigate(`/patches/${id}`);
  };

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>{eyebrow}</span>
          <h1 className={styles.pageTitle}>Patches</h1>
        </div>
        <span className={styles.headSpacer} />
      </div>

      <div className={styles.toolbar}>
        <div className={styles.searchBox}>
          <Icons.IconSearch className={styles.searchIcon} size={14} />
          <input
            type="text"
            placeholder="Search patches…"
            value={searchValue}
            onChange={(e) => onSearchChange(e.target.value)}
            aria-label="Search patches"
            data-testid="patches-search"
          />
          <Kbd>/</Kbd>
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

      <div className={styles.body}>
        {isLoading && patches.length === 0 && <div className={styles.empty}>Loading patches…</div>}

        {!isLoading && patches.length === 0 && (
          <div className={styles.empty}>No patches match the current filters.</div>
        )}

        {patches.length > 0 && isMobile && (
          <div className={styles.mobileList}>
            {patches.map((rec) => (
              <PatchRailRow key={rec.patch_id} record={rec} linkSearch="?from=dashboard" />
            ))}
          </div>
        )}

        {patches.length > 0 && !isMobile && (
          <div className={styles.tableWrap}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th className={styles.colTitle}>Title</th>
                  <th className={styles.colStatus}>Status</th>
                  <th className={styles.colAuthor}>Author</th>
                  <th className={styles.colRepo}>Repo</th>
                  <th className={styles.colUpdated}>Updated</th>
                  <th className={styles.colReviews}>Reviews</th>
                </tr>
              </thead>
              <tbody>
                {patches.map((rec) => {
                  const p = rec.patch;
                  const status =
                    p.status === "Open" && p.review_summary.approved
                      ? "approved"
                      : normalizePatchStatus(p.status);
                  return (
                    <tr
                      key={rec.patch_id}
                      data-testid={`patches-list-row-${rec.patch_id}`}
                      onClick={() => handleRowClick(rec.patch_id)}
                    >
                      <td className={styles.colTitle}>
                        <div className={styles.titleCell}>
                          <span className={styles.titleText}>{p.title || "(untitled)"}</span>
                        </div>
                      </td>
                      <td className={styles.colStatus}>
                        <Badge status={status} />
                      </td>
                      <td className={styles.colAuthor}>
                        <span className={styles.author}>
                          <Avatar name={p.creator} size="md" />
                          <span className={styles.authorName}>{p.creator}</span>
                        </span>
                      </td>
                      <td className={styles.colRepo}>
                        <PatchRepoLink patch={p} />
                      </td>
                      <td className={styles.colUpdated}>
                        <AgoTime iso={rec.timestamp} />
                      </td>
                      <td className={styles.colReviews}>
                        {p.review_summary.count > 0 ? (
                          <span
                            className={`${styles.reviewCount}${p.review_summary.approved ? ` ${styles.reviewApproved}` : ""}`}
                          >
                            {p.review_summary.count}
                            {p.review_summary.approved ? " ✓" : ""}
                          </span>
                        ) : (
                          <span className={styles.dash}>—</span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
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
      </div>
    </div>
  );
}
