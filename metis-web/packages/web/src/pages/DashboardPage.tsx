import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { usePaginatedIssues, useIssueCount, type IssueFilters } from "../features/issues/usePaginatedIssues";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssueFilterSidebar, LABEL_FILTER_PREFIX } from "../features/dashboard/IssueFilterSidebar";
import { HeterogeneousItemList } from "../features/dashboard/HeterogeneousItemList";
import type { WorkItem } from "../features/dashboard/workItemTypes";
import { usePageIssueTrees } from "../features/dashboard/usePageIssueTrees";
import { useActiveSessionIssueIds } from "../features/dashboard/useActiveSessionIssueIds";
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
  // For "everything" (null), no extra server filters

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

  const isInboxFilter = filterRootId === "inbox";

  // Build server-side filters for the paginated query
  const serverFilters = useMemo(
    () => buildServerFilters(filterRootId, username, inboxLabelId, searchQuery),
    [filterRootId, username, inboxLabelId, searchQuery],
  );

  const {
    data: paginatedData,
    isLoading: paginatedLoading,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedIssues(serverFilters);

  // Flatten paginated pages into a single array
  const issues = useMemo(() => {
    return paginatedData?.pages.flatMap((page) => page.issues) ?? [];
  }, [paginatedData]);

  const isLoading = paginatedLoading;

  // Badge count queries (count-only, no issue data fetched).
  // Uses multi-status filter to get open + in-progress counts in a single call
  // per category, excluding terminal statuses (closed, failed, dropped, rejected).
  const inboxCountFilters = useMemo<IssueFilters>(() => {
    if (!inboxLabelId || !username) return {};
    return { labels: inboxLabelId, creator: username, status: "open,in-progress" };
  }, [inboxLabelId, username]);

  const myIssuesCountFilters = useMemo<IssueFilters>(() => {
    if (!username) return {};
    return { creator: username, status: "open,in-progress" };
  }, [username]);

  const inboxEnabled = !!inboxLabelId && !!username;
  const { data: inboxTotalCount = 0 } = useIssueCount(inboxCountFilters, inboxEnabled);

  const myIssuesEnabled = !!username;
  const { data: myIssuesTotalCount = 0 } = useIssueCount(myIssuesCountFilters, myIssuesEnabled);

  // Fetch active session IDs to exclude from badge counts.
  // Issues with running/pending sessions should not count toward badges.
  const { activeIssueIds } = useActiveSessionIssueIds();
  const activeIdsParam = useMemo(
    () => (activeIssueIds.size > 0 ? Array.from(activeIssueIds).join(",") : null),
    [activeIssueIds],
  );

  // Count how many active-session issues match each badge filter so we can subtract them.
  // Uses multi-status filter to match the total queries above.
  const inboxActiveFilters = useMemo<IssueFilters>(() => {
    if (!activeIdsParam || !inboxLabelId || !username) return {};
    return { labels: inboxLabelId, creator: username, ids: activeIdsParam, status: "open,in-progress" };
  }, [activeIdsParam, inboxLabelId, username]);

  const myIssuesActiveFilters = useMemo<IssueFilters>(() => {
    if (!activeIdsParam || !username) return {};
    return { creator: username, ids: activeIdsParam, status: "open,in-progress" };
  }, [activeIdsParam, username]);

  const activeCountEnabled = !!activeIdsParam;
  const { data: inboxActiveCount = 0 } = useIssueCount(inboxActiveFilters, activeCountEnabled && inboxEnabled);
  const { data: myIssuesActiveCount = 0 } = useIssueCount(myIssuesActiveFilters, activeCountEnabled && myIssuesEnabled);

  const inboxCount = Math.max(0, inboxTotalCount - inboxActiveCount);
  const myIssuesCount = Math.max(0, myIssuesTotalCount - myIssuesActiveCount);

  const assignees = useMemo(() => {
    const set = new Set<string>();
    for (const record of issues) {
      if (record.issue.assignee) set.add(record.issue.assignee);
    }
    return Array.from(set).sort();
  }, [issues]);

  // Build flat work items from issues (no tree traversal needed;
  // usePageIssueTrees provides supplementary tree data)
  const allWorkItems = useMemo((): WorkItem[] => {
    return issues.map((issue) => ({
      kind: "issue" as const,
      id: issue.issue_id,
      data: issue,
      lastUpdated: issue.timestamp,
      isTerminal: TERMINAL_STATUSES.has(issue.issue.status),
    }));
  }, [issues]);

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
          isLoading={isLoading}
          treeLoading={pageTreeLoading}
          sidebarCollapsed={sidebarCollapsed}
          onToggleSidebar={handleToggleSidebar}
          onToggleDrawer={handleToggleDrawer}
          filterRootId={filterRootId}
          searchValue={searchValue}
          onSearchChange={handleSearchChange}
          inboxLabelId={isInboxFilter && inboxLabel ? inboxLabel.label_id : undefined}
          hasNextPage={hasNextPage ?? false}
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
