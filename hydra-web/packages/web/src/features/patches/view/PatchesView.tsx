import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, Badge, Icons, Kbd } from "@hydra/ui";
import type { PatchStatus, PatchSummaryRecord } from "@hydra/api";
import { usePaginatedPatches, usePatchCount } from "../../dashboard/usePaginatedPatches";
import { normalizePatchStatus } from "../../../utils/statusMapping";
import { useMediaQuery } from "../../../hooks/useMediaQuery";
import { PatchRailRow } from "../../related/RailRow";
import styles from "./PatchesView.module.css";

const MOBILE_QUERY = "(max-width: 768px)";

interface StatusFilter {
  key: "all" | PatchStatus;
  label: string;
}

const STATUS_FILTERS: StatusFilter[] = [
  { key: "all", label: "All" },
  { key: "Open", label: "Open" },
  { key: "ChangesRequested", label: "Changes requested" },
  { key: "Merged", label: "Merged" },
  { key: "Closed", label: "Closed" },
];

function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "";
  const sec = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (sec < 60) return "now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day}d`;
  const mo = Math.floor(day / 30);
  return `${mo}mo`;
}

export function PatchesView() {
  const navigate = useNavigate();
  const isMobile = useMediaQuery(MOBILE_QUERY);
  const [searchValue, setSearchValue] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const [selectedStatus, setSelectedStatus] = useState<PatchStatus | null>(null);

  useEffect(() => {
    return () => clearTimeout(debounceRef.current);
  }, []);

  const handleSearchChange = (value: string) => {
    setSearchValue(value);
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => setSearchQuery(value), 300);
  };

  const filters = useMemo(
    () => ({
      q: searchQuery || undefined,
      status: selectedStatus ? [selectedStatus] : undefined,
    }),
    [searchQuery, selectedStatus],
  );

  const { data, isLoading, fetchNextPage, hasNextPage, isFetchingNextPage } =
    usePaginatedPatches(filters);

  const { data: totalCount } = usePatchCount(filters);

  const patches = useMemo<PatchSummaryRecord[]>(() => {
    const seen = new Set<string>();
    return (data?.pages.flatMap((p) => p.patches) ?? []).filter((rec) => {
      if (seen.has(rec.patch_id)) return false;
      seen.add(rec.patch_id);
      return true;
    });
  }, [data]);

  const handleRowClick = (id: string) => {
    navigate(`/patches/${id}`);
  };

  const activeKey: StatusFilter["key"] = selectedStatus ?? "all";
  const displayCount = totalCount ?? patches.length;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>
            WORK · {displayCount === 1 ? "1 PATCH" : `${displayCount} PATCHES`}
          </span>
          <h1 className={styles.pageTitle}>Patches</h1>
        </div>
        <span className={styles.headSpacer} />
      </div>

      <div className={styles.toolbar}>
        {STATUS_FILTERS.map((f) => (
          <button
            key={f.key}
            type="button"
            className={`${styles.chipFilter}${activeKey === f.key ? ` ${styles.chipFilterActive}` : ""}`}
            onClick={() => setSelectedStatus(f.key === "all" ? null : f.key)}
            data-testid={`patches-filter-${f.key}`}
          >
            <span>{f.label}</span>
          </button>
        ))}
        <span className={styles.toolbarSpacer} />
        <div className={styles.searchBox}>
          <Icons.IconSearch className={styles.searchIcon} size={14} />
          <input
            type="text"
            placeholder="Search patches…"
            value={searchValue}
            onChange={(e) => handleSearchChange(e.target.value)}
            aria-label="Search patches"
            data-testid="patches-search"
          />
          <Kbd>/</Kbd>
        </div>
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
                  const authorKind = p.created_by ? "agent" : "human";
                  return (
                    <tr key={rec.patch_id} onClick={() => handleRowClick(rec.patch_id)}>
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
                          <Avatar name={p.creator} kind={authorKind} size="md" />
                          <span className={styles.authorName}>{p.creator}</span>
                        </span>
                      </td>
                      <td className={styles.colRepo}>{p.service_repo_name}</td>
                      <td className={styles.colUpdated}>{relativeTime(rec.timestamp)}</td>
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
              onClick={() => fetchNextPage()}
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
