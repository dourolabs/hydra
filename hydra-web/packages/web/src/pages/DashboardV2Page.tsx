import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import {
  usePaginatedIssues,
  useIssueCount,
  type IssueFilters,
} from "../features/issues/usePaginatedIssues";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssueFilterSidebar } from "../features/dashboard-v2/IssueFilterSidebar";
import { HeterogeneousItemList } from "../features/dashboard-v2/HeterogeneousItemList";
import type { WorkItem } from "../features/dashboard-v2/workItemTypes";
import { usePageIssueTrees } from "../features/dashboard-v2/usePageIssueTrees";
import { TERMINAL_STATUSES } from "../utils/statusMapping";
import { readCollapsed, writeCollapsed } from "../features/dashboard-v2/sidebarStorage";
import { IssueCreateModal } from "../features/dashboard-v2/IssueCreateModal";
import { useInboxLabel } from "../features/labels/useLabels";
import styles from "./DashboardV2Page.module.css";

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

  if (filterRootId === "your-issues") {
    if (inboxLabelId) filters.labels = inboxLabelId;
    if (username) filters.creator = username;
  } else if (filterRootId === "assigned") {
    if (username) filters.assignee = username;
  }

  return filters;
}

export function DashboardV2Page() {
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
    selectedParam === "your-issues" || selectedParam === "assigned"
      ? selectedParam
      : "your-issues",
  );
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readCollapsed);
  const [drawerOpen, setDrawerOpen] = useState(false);

  const username = user ? actorDisplayName(user.actor) : "";
  const { data: inboxLabel } = useInboxLabel();
  const inboxLabelId = inboxLabel?.label_id;

  const isInboxFilter = filterRootId === "your-issues";

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

  // Badge count query for "Assigned to You" — only open status
  const assignedCountFilters = useMemo<IssueFilters>(() => {
    if (!username) return {};
    return { assignee: username, status: "open" };
  }, [username]);

  const assignedEnabled = !!username;
  const { data: assignedCount = 0 } = useIssueCount(assignedCountFilters, assignedEnabled);

  const assignees = useMemo(() => {
    const set = new Set<string>();
    for (const record of issues) {
      if (record.issue.assignee) set.add(record.issue.assignee);
    }
    return Array.from(set).sort();
  }, [issues]);

  // Per-issue tree construction via relationships API
  const {
    isActiveMap,
    childStatusMap,
    sessionsByIssue,
    isLoading: pageTreeLoading,
  } = usePageIssueTrees(issues, username);

  // Build flat work items from issues only
  const allWorkItems = useMemo((): WorkItem[] => {
    return issues.map((issue) => ({
      kind: "issue" as const,
      id: issue.issue_id,
      data: issue,
      lastUpdated: issue.timestamp,
      isTerminal: TERMINAL_STATUSES.has(issue.issue.status),
    }));
  }, [issues]);

  useEffect(() => {
    if (!searchParams.has("selected")) {
      setSearchParams(
        (prev) => {
          prev.set("selected", "your-issues");
          return prev;
        },
        { replace: true },
      );
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleFilterChange = useCallback(
    (rootId: string | null) => {
      setFilterRootId(rootId);
      setSearchParams(
        (prev) => {
          prev.set("selected", rootId ?? "your-issues");
          return prev;
        },
        { replace: true },
      );
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

  return (
    <div className={styles.page}>
      <div className={styles.dashboardRow}>
        <IssueFilterSidebar
          activeFilter={filterRootId}
          onFilterChange={handleFilterChange}
          collapsed={sidebarCollapsed}
          drawerOpen={drawerOpen}
          onDrawerClose={handleDrawerClose}
          assignedCount={assignedCount}
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
