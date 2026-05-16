import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import type { IssueStatus } from "@hydra/api";
import {
  usePaginatedIssues,
  type IssueFilters,
} from "../features/issues/usePaginatedIssues";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssuesView } from "../features/issues/view/IssuesView";
import { usePageIssueTrees } from "../features/dashboard/usePageIssueTrees";
import { readFilterState, writeFilterState } from "../features/dashboard/filterStorage";
import { useInboxLabel } from "../features/labels/useLabels";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./IssuesListPage.module.css";

const VALID_FILTERS = ["your-issues", "assigned", "all", "in_progress"];

function buildServerFilters(
  filterRootId: string | null,
  username: string,
  inboxLabelId: string | undefined,
  searchQuery: string,
  selectedIssueStatus: IssueStatus | null,
  selectedLabelId: string | null,
): IssueFilters {
  const filters: IssueFilters = {};

  if (searchQuery) filters.q = searchQuery;

  if (filterRootId === "your-issues") {
    if (inboxLabelId) filters.labels = inboxLabelId;
    if (username) filters.creator = username;
  } else if (filterRootId === "assigned") {
    if (username) filters.assignee = username;
  } else if (filterRootId === "in_progress") {
    filters.status = "in-progress";
  }

  if (selectedIssueStatus) {
    filters.status = selectedIssueStatus;
  }

  if (selectedLabelId) {
    filters.labels = filters.labels ? `${filters.labels},${selectedLabelId}` : selectedLabelId;
  }

  return filters;
}

function eyebrowFor(filterRootId: string | null, count: number): string {
  const n = count === 1 ? "1 ISSUE" : `${count} ISSUES`;
  switch (filterRootId) {
    case "assigned":
      return `ASSIGNED · ${n}`;
    case "in_progress":
      return `IN PROGRESS · ${n}`;
    case "all":
      return `ALL · ${n}`;
    case "your-issues":
    default:
      return `WORK · ${n}`;
  }
}

function titleFor(filterRootId: string | null): string {
  switch (filterRootId) {
    case "assigned":
      return "Assigned to me";
    case "in_progress":
      return "In progress";
    case "all":
      return "All issues";
    case "your-issues":
    default:
      return "Issues";
  }
}

export function IssuesListPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const selectedParam = searchParams.get("selected");
  useBreadcrumbs([{ label: "Workspace", to: "/" }], titleFor(selectedParam));
  const { user } = useAuth();
  const savedFilters = useMemo(() => readFilterState(), []);
  const [searchValue, setSearchValue] = useState(savedFilters?.searchValue ?? "");
  const [searchQuery, setSearchQuery] = useState(savedFilters?.searchValue ?? "");
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const handleSearchChange = useCallback((value: string) => {
    setSearchValue(value);
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      setSearchQuery(value);
    }, 300);
  }, []);

  useEffect(() => {
    return () => clearTimeout(debounceRef.current);
  }, []);

  const labelParam = searchParams.get("label");

  const [filterRootId, setFilterRootId] = useState<string | null>(() => {
    if (selectedParam && VALID_FILTERS.includes(selectedParam)) return selectedParam;
    if (savedFilters && VALID_FILTERS.includes(savedFilters.filterRootId))
      return savedFilters.filterRootId;
    return "your-issues";
  });
  const [selectedIssueStatus, setSelectedIssueStatus] = useState<IssueStatus | null>(
    savedFilters?.selectedIssueStatus ?? null,
  );
  const [selectedLabelId] = useState<string | null>(() => {
    if (labelParam) return labelParam;
    return savedFilters?.selectedLabelId ?? null;
  });

  useEffect(() => {
    if (selectedParam && VALID_FILTERS.includes(selectedParam)) {
      setFilterRootId((current) => (current === selectedParam ? current : selectedParam));
    }
  }, [selectedParam]);

  useEffect(() => {
    writeFilterState({
      filterRootId: filterRootId ?? "your-issues",
      selectedIssueStatus,
      selectedPatchStatus: null,
      selectedLabelId,
      searchValue,
    });
  }, [filterRootId, selectedIssueStatus, selectedLabelId, searchValue]);

  const username = user ? actorDisplayName(user.actor) : "";
  const { data: inboxLabel } = useInboxLabel();
  const inboxLabelId = inboxLabel?.label_id;

  const serverFilters = useMemo(
    () =>
      buildServerFilters(
        filterRootId,
        username,
        inboxLabelId,
        searchQuery,
        selectedIssueStatus,
        selectedLabelId,
      ),
    [filterRootId, username, inboxLabelId, searchQuery, selectedIssueStatus, selectedLabelId],
  );

  const {
    data: paginatedData,
    isLoading,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedIssues(serverFilters, true);

  const issues = useMemo(() => {
    const seen = new Set<string>();
    return (paginatedData?.pages.flatMap((page) => page.issues) ?? []).filter((issue) => {
      if (seen.has(issue.issue_id)) return false;
      seen.add(issue.issue_id);
      return true;
    });
  }, [paginatedData]);

  const { childStatusMap } = usePageIssueTrees(issues, username);

  // Normalise any legacy `?selected=…` URLs the sidebar no longer produces
  // (`patches`, `documents`) back to the default filter without forcing a
  // history entry.
  useEffect(() => {
    if (selectedParam && !VALID_FILTERS.includes(selectedParam)) {
      setSearchParams(
        (prev) => {
          prev.delete("selected");
          return prev;
        },
        { replace: true },
      );
    }
  }, [selectedParam, setSearchParams]);

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  return (
    <div className={styles.page}>
      <IssuesView
        issues={issues}
        childStatusMap={childStatusMap}
        isLoading={isLoading}
        filterRootId={filterRootId}
        hasNextPage={hasNextPage ?? false}
        isFetchingNextPage={isFetchingNextPage ?? false}
        onLoadMore={handleLoadMore}
        searchValue={searchValue}
        onSearchChange={handleSearchChange}
        selectedStatus={selectedIssueStatus}
        onStatusChange={setSelectedIssueStatus}
        eyebrow={eyebrowFor(filterRootId, issues.length)}
        title={titleFor(filterRootId)}
      />
    </div>
  );
}
