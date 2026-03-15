import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { useIssues } from "../features/issues/useIssues";
import { usePaginatedIssues, useIssueCount, type IssueFilters } from "../features/issues/usePaginatedIssues";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssueFilterSidebar, LABEL_FILTER_PREFIX } from "../features/dashboard/IssueFilterSidebar";
import { HeterogeneousItemList } from "../features/dashboard/HeterogeneousItemList";
import { useTransitiveWorkItems, type WorkItem } from "../features/dashboard/useTransitiveWorkItems";
import { usePageIssueTrees } from "../features/dashboard/usePageIssueTrees";
import { TERMINAL_STATUSES } from "../utils/statusMapping";
import { readCollapsed, writeCollapsed } from "../features/dashboard/sidebarStorage";
import { IssueCreateModal } from "../features/dashboard/IssueCreateModal";
import { useInboxLabel } from "../features/labels/useLabels";
import styles from "./DashboardPage.module.css";

/** Build server-side IssueFilters from the current filter selection. */
function buildServerFilters(
  filterRootId: string | null,
  username: string,
  inboxLabelId: string | undefined,
  searchQuery: string,
): IssueFilters {
  const filters: IssueFilters = {};

  if (searchQuery) {
    filters.q = searchQuery;
  }

  if (filterRootId === "inbox") {
    if (inboxLabelId) filters.labels = inboxLabelId;
    if (username) filters.creator = username;
  } else if (filterRootId === "my-issues") {
    if (username) filters.creator = username;
  } else if (filterRootId?.startsWith(LABEL_FILTER_PREFIX)) {
    const labelId = filterRootId.slice(LABEL_FILTER_PREFIX.length);
    filters.labels = labelId;
  }
  // For "everything" (null) or specific issue root IDs, no extra server filters

  return filters;
}

export function DashboardPage() {
  const { user } = useAuth();
  const [searchValue, setSearchValue] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
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

  const [searchParams, setSearchParams] = useSearchParams();
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const selectedParam = searchParams.get("selected");
  const [filterRootId, setFilterRootId] = useState<string | null>(
    selectedParam === "everything" ? null : (selectedParam ?? "inbox"),
  );
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readCollapsed);
  const [drawerOpen, setDrawerOpen] = useState(false);

  const username = user ? actorDisplayName(user.actor) : "";
  const { data: inboxLabel } = useInboxLabel();
  const inboxLabelId = inboxLabel?.label_id;

  // Determine if the current filter is a "special" filter (inbox, my-issues, label)
  // vs. a specific issue root or "everything"
  const isLabelFilter = filterRootId?.startsWith(LABEL_FILTER_PREFIX) ?? false;
  const isMyIssuesFilter = filterRootId === "my-issues";
  const isInboxFilter = filterRootId === "inbox";
  const isSpecialFilter = isInboxFilter || isMyIssuesFilter || isLabelFilter;

  // Build server-side filters for the paginated query
  const serverFilters = useMemo(
    () => buildServerFilters(filterRootId, username, inboxLabelId, searchQuery),
    [filterRootId, username, inboxLabelId, searchQuery],
  );

  // For special filters (inbox, my-issues, label) and "everything", use paginated query
  // For specific issue roots, fall back to the old useIssues hook
  const hookRootId = isSpecialFilter ? null : filterRootId;
  const usePaginated = isSpecialFilter || filterRootId === null;

  const {
    data: paginatedData,
    isLoading: paginatedLoading,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedIssues(serverFilters, usePaginated);

  // Fall back to old hook for specific issue root selections
  const { data: legacyIssues, isLoading: legacyLoading } = useIssues(
    !usePaginated ? (searchQuery || undefined) : undefined,
    !usePaginated,
  );

  // Flatten paginated pages into a single array
  const issues = useMemo(() => {
    if (usePaginated) {
      return paginatedData?.pages.flatMap((page) => page.issues) ?? [];
    }
    return legacyIssues ?? [];
  }, [usePaginated, paginatedData, legacyIssues]);

  const isLoading = usePaginated ? paginatedLoading : legacyLoading;

  // Badge count queries (count-only, no issue data fetched).
  // The backend status filter accepts a single status, so we issue separate
  // queries for "open" and "in-progress" to exclude terminal statuses
  // (closed, failed, dropped, rejected) which should not inflate badge counts.
  const inboxOpenFilters = useMemo<IssueFilters>(() => {
    if (!inboxLabelId || !username) return {};
    return { labels: inboxLabelId, creator: username, status: "open" };
  }, [inboxLabelId, username]);

  const inboxInProgressFilters = useMemo<IssueFilters>(() => {
    if (!inboxLabelId || !username) return {};
    return { labels: inboxLabelId, creator: username, status: "in-progress" };
  }, [inboxLabelId, username]);

  const myIssuesOpenFilters = useMemo<IssueFilters>(() => {
    if (!username) return {};
    return { creator: username, status: "open" };
  }, [username]);

  const myIssuesInProgressFilters = useMemo<IssueFilters>(() => {
    if (!username) return {};
    return { creator: username, status: "in-progress" };
  }, [username]);

  const inboxEnabled = !!inboxLabelId && !!username;
  const { data: inboxOpenCount = 0 } = useIssueCount(inboxOpenFilters, inboxEnabled);
  const { data: inboxInProgressCount = 0 } = useIssueCount(inboxInProgressFilters, inboxEnabled);
  const inboxCount = inboxOpenCount + inboxInProgressCount;

  const myIssuesEnabled = !!username;
  const { data: myIssuesOpenCount = 0 } = useIssueCount(myIssuesOpenFilters, myIssuesEnabled);
  const { data: myIssuesInProgressCount = 0 } = useIssueCount(myIssuesInProgressFilters, myIssuesEnabled);
  const myIssuesCount = myIssuesOpenCount + myIssuesInProgressCount;

  const assignees = useMemo(() => {
    const set = new Set<string>();
    for (const record of issues) {
      if (record.issue.assignee) set.add(record.issue.assignee);
    }
    return Array.from(set).sort();
  }, [issues]);

  // For server-filtered results (special filters), create work items directly
  // without tree traversal since the server already filtered for us.
  // For specific issue roots ("everything" or a specific issue), use tree traversal.
  const { items: treeWorkItems, isLoading: treeLoading } =
    useTransitiveWorkItems(isSpecialFilter ? null : hookRootId, isSpecialFilter ? [] : issues);

  const flatWorkItems = useMemo((): WorkItem[] => {
    if (!isSpecialFilter) return [];
    return issues.map((issue) => ({
      kind: "issue" as const,
      id: issue.issue_id,
      data: issue,
      lastUpdated: issue.timestamp,
      isTerminal: TERMINAL_STATUSES.has(issue.issue.status),
    }));
  }, [isSpecialFilter, issues]);

  const allWorkItems = isSpecialFilter ? flatWorkItems : treeWorkItems;
  const workItemsLoading = isSpecialFilter ? false : treeLoading;

  // Per-issue tree construction via relationships API
  const {
    isActiveMap,
    childStatusMap,
    sessionsByIssue,
    isLoading: pageTreeLoading,
  } = usePageIssueTrees(issues, username);

  useEffect(() => {
    if (!searchParams.has("selected")) {
      setSearchParams((prev) => {
        prev.set("selected", "inbox");
        return prev;
      }, { replace: true });
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleFilterChange = useCallback(
    (rootId: string | null) => {
      setFilterRootId(rootId);
      setSearchParams((prev) => {
        prev.set("selected", rootId ?? "everything");
        return prev;
      }, { replace: true });
    },
    [setSearchParams],
  );

  const handleToggleSidebar = useCallback(() => {
    const next = !sidebarCollapsed;
    writeCollapsed(next);
    setSidebarCollapsed(next);
  }, [sidebarCollapsed]);

  const handleToggleDrawer = useCallback(() => {
    setDrawerOpen((v) => !v);
  }, []);

  const handleDrawerClose = useCallback(() => {
    setDrawerOpen(false);
  }, []);

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) {
      fetchNextPage();
    }
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  if (isLoading && !issues.length) {
    return (
      <div className={styles.center}>
        <Spinner size="lg" />
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <div className={styles.dashboardRow}>
        <IssueFilterSidebar
          allIssues={issues}
          activeFilter={filterRootId}
          onFilterChange={handleFilterChange}
          collapsed={sidebarCollapsed}
          drawerOpen={drawerOpen}
          onDrawerClose={handleDrawerClose}
          isActiveMap={isActiveMap}
          username={username}
          inboxCount={inboxCount}
          myIssuesCount={myIssuesCount}
        />
        <HeterogeneousItemList
          items={allWorkItems}
          sessionsByIssue={sessionsByIssue}
          childStatusMap={childStatusMap}
          isActiveMap={isActiveMap}
          isLoading={workItemsLoading}
          treeLoading={pageTreeLoading}
          sidebarCollapsed={sidebarCollapsed}
          onToggleSidebar={handleToggleSidebar}
          onToggleDrawer={handleToggleDrawer}
          filterRootId={filterRootId}
          searchValue={searchValue}
          onSearchChange={handleSearchChange}
          inboxLabelId={isInboxFilter && inboxLabel ? inboxLabel.label_id : undefined}
          hasNextPage={usePaginated && (hasNextPage ?? false)}
          isFetchingNextPage={isFetchingNextPage}
          onLoadMore={handleLoadMore}
        />
      </div>
      <button
        type="button"
        className={styles.createButton}
        onClick={() => setCreateModalOpen(true)}
      >
        + Create Issue
      </button>
      <IssueCreateModal
        open={createModalOpen}
        onClose={() => setCreateModalOpen(false)}
        assignees={assignees}
      />
    </div>
  );
}
