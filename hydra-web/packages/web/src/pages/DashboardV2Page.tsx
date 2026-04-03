import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import type { IssueStatus, PatchStatus } from "@hydra/api";
import {
  usePaginatedIssues,
  useIssueCount,
  type IssueFilters,
} from "../features/issues/usePaginatedIssues";
import { usePaginatedPatches } from "../features/dashboard-v2/usePaginatedPatches";
import { usePaginatedDocuments } from "../features/dashboard-v2/usePaginatedDocuments";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssueFilterSidebar } from "../features/dashboard-v2/IssueFilterSidebar";
import { HeterogeneousItemList } from "../features/dashboard-v2/HeterogeneousItemList";
import { FilterBar } from "../features/dashboard-v2/FilterBar";
import type { WorkItem } from "../features/dashboard-v2/workItemTypes";
import { usePageIssueTrees } from "../features/dashboard-v2/usePageIssueTrees";
import { TERMINAL_STATUSES } from "../utils/statusMapping";
import { readCollapsed, writeCollapsed } from "../features/dashboard-v2/sidebarStorage";
import { readFilterState, writeFilterState } from "../features/dashboard-v2/filterStorage";
import { IssueCreateModal } from "../features/dashboard-v2/IssueCreateModal";
import { useInboxLabel } from "../features/labels/useLabels";
import styles from "./DashboardV2Page.module.css";

const VALID_FILTERS = ["your-issues", "assigned", "all", "patches", "documents"];

const ALL_ISSUE_STATUSES: IssueStatus[] = [
  "open", "in-progress", "closed", "dropped", "rejected", "failed",
];
const ALL_PATCH_STATUSES: PatchStatus[] = [
  "Open", "Closed", "Merged", "ChangesRequested",
];

/** Build server-side IssueFilters from the current filter selection. */
function buildServerFilters(
  filterRootId: string | null,
  username: string,
  inboxLabelId: string | undefined,
  searchQuery: string,
  issueStatuses: Set<IssueStatus>,
  selectedLabelId: string | null,
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
  } else if (filterRootId === "all") {
    // No additional filters — show all issues
  }

  // Apply status filter only when not all statuses are selected
  if (issueStatuses.size > 0 && issueStatuses.size < ALL_ISSUE_STATUSES.length) {
    filters.status = [...issueStatuses].join(",");
  }

  // Apply label filter
  if (selectedLabelId) {
    // Append to existing labels if present (e.g. inbox label), otherwise set
    filters.labels = filters.labels
      ? `${filters.labels},${selectedLabelId}`
      : selectedLabelId;
  }

  return filters;
}

export function DashboardV2Page() {
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

  const [searchParams, setSearchParams] = useSearchParams();
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const selectedParam = searchParams.get("selected");
  const [filterRootId, setFilterRootId] = useState<string | null>(() => {
    // URL param takes priority over localStorage
    if (selectedParam && VALID_FILTERS.includes(selectedParam)) return selectedParam;
    if (savedFilters && VALID_FILTERS.includes(savedFilters.filterRootId)) return savedFilters.filterRootId;
    return "your-issues";
  });
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readCollapsed);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [selectedIssueStatuses, setSelectedIssueStatuses] = useState<Set<IssueStatus>>(
    () => new Set(savedFilters?.selectedIssueStatuses ?? ALL_ISSUE_STATUSES),
  );
  const [selectedPatchStatuses, setSelectedPatchStatuses] = useState<Set<PatchStatus>>(
    () => new Set(savedFilters?.selectedPatchStatuses ?? ALL_PATCH_STATUSES),
  );
  const [selectedLabelId, setSelectedLabelId] = useState<string | null>(
    savedFilters?.selectedLabelId ?? null,
  );

  // Persist filter state to localStorage
  useEffect(() => {
    writeFilterState({
      filterRootId: filterRootId ?? "your-issues",
      selectedIssueStatuses: Array.from(selectedIssueStatuses),
      selectedPatchStatuses: Array.from(selectedPatchStatuses),
      selectedLabelId,
      searchValue,
    });
  }, [filterRootId, selectedIssueStatuses, selectedPatchStatuses, selectedLabelId, searchValue]);

  const username = user ? actorDisplayName(user.actor) : "";
  const { data: inboxLabel } = useInboxLabel();
  const inboxLabelId = inboxLabel?.label_id;

  const isInboxFilter = filterRootId === "your-issues";
  const isArtifactFilter = filterRootId === "patches" || filterRootId === "documents";

  // Build server-side filters for the paginated query
  const serverFilters = useMemo(
    () => buildServerFilters(filterRootId, username, inboxLabelId, searchQuery, selectedIssueStatuses, selectedLabelId),
    [filterRootId, username, inboxLabelId, searchQuery, selectedIssueStatuses, selectedLabelId],
  );

  const isSearching = searchQuery.length > 0;

  // When searching, use a global filter (only search query, no sidebar filters)
  const searchOnlyFilters = useMemo<IssueFilters>(
    () => (searchQuery ? { q: searchQuery } : {}),
    [searchQuery],
  );

  const {
    data: paginatedData,
    isLoading: paginatedLoading,
    fetchNextPage: fetchNextIssues,
    hasNextPage: hasNextIssues,
    isFetchingNextPage: isFetchingNextIssues,
  } = usePaginatedIssues(isSearching ? searchOnlyFilters : serverFilters, !isArtifactFilter || isSearching);

  const patchFilters = useMemo(() => {
    const pf: { q?: string; status?: PatchStatus[] } = {};
    if (searchQuery) pf.q = searchQuery;
    // Only apply status filter when not all are selected and on the patches tab (not searching)
    if (!isSearching && selectedPatchStatuses.size > 0 && selectedPatchStatuses.size < ALL_PATCH_STATUSES.length) {
      pf.status = [...selectedPatchStatuses];
    }
    return pf;
  }, [searchQuery, isSearching, selectedPatchStatuses]);

  const {
    data: patchesData,
    isLoading: patchesLoading,
    fetchNextPage: fetchNextPatches,
    hasNextPage: hasNextPatches,
    isFetchingNextPage: isFetchingNextPatches,
  } = usePaginatedPatches(patchFilters, filterRootId === "patches" || isSearching);

  const {
    data: documentsData,
    isLoading: documentsLoading,
    fetchNextPage: fetchNextDocuments,
    hasNextPage: hasNextDocuments,
    isFetchingNextPage: isFetchingNextDocuments,
  } = usePaginatedDocuments(searchQuery, filterRootId === "documents" || isSearching);

  // Flatten paginated pages into a single array
  const issues = useMemo(() => {
    const seen = new Set<string>();
    return (paginatedData?.pages.flatMap((page) => page.issues) ?? []).filter((issue) => {
      if (seen.has(issue.issue_id)) return false;
      seen.add(issue.issue_id);
      return true;
    });
  }, [paginatedData]);

  const isLoading = isSearching
    ? paginatedLoading || patchesLoading || documentsLoading
    : isArtifactFilter
      ? (filterRootId === "patches" ? patchesLoading : documentsLoading)
      : paginatedLoading;

  const hasNextPage = isSearching
    ? (hasNextIssues || hasNextPatches || hasNextDocuments)
    : isArtifactFilter
      ? (filterRootId === "patches" ? hasNextPatches : hasNextDocuments)
      : hasNextIssues;

  const isFetchingNextPage = isSearching
    ? (isFetchingNextIssues || isFetchingNextPatches || isFetchingNextDocuments)
    : isArtifactFilter
      ? (filterRootId === "patches" ? isFetchingNextPatches : isFetchingNextDocuments)
      : isFetchingNextIssues;

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

  // Build flat work items from issues, patches, or documents
  const allWorkItems = useMemo((): WorkItem[] => {
    // When searching, merge results from all three entity types
    if (isSearching) {
      const issueItems: WorkItem[] = issues.map((issue) => ({
        kind: "issue" as const,
        id: issue.issue_id,
        data: issue,
        lastUpdated: issue.timestamp,
        isTerminal: TERMINAL_STATUSES.has(issue.issue.status),
      }));
      const patchItems: WorkItem[] = (patchesData?.pages.flatMap((page) => page.patches) ?? []).map((patch) => ({
        kind: "patch" as const,
        id: patch.patch_id,
        data: patch,
        lastUpdated: patch.timestamp,
        isTerminal: patch.patch.status === "Merged" || patch.patch.status === "Closed",
        sourceIssueId: undefined,
      }));
      const docItems: WorkItem[] = (documentsData?.pages.flatMap((page) => page.documents) ?? []).map((doc) => ({
        kind: "document" as const,
        id: doc.document_id,
        data: doc,
        lastUpdated: doc.timestamp,
        isTerminal: false,
        sourceIssueId: undefined,
      }));
      return [...issueItems, ...patchItems, ...docItems];
    }

    if (filterRootId === "patches") {
      const patches = patchesData?.pages.flatMap((page) => page.patches) ?? [];
      return patches.map((patch) => ({
        kind: "patch" as const,
        id: patch.patch_id,
        data: patch,
        lastUpdated: patch.timestamp,
        isTerminal: patch.patch.status === "Merged" || patch.patch.status === "Closed",
        sourceIssueId: undefined,
      }));
    }
    if (filterRootId === "documents") {
      const documents = documentsData?.pages.flatMap((page) => page.documents) ?? [];
      return documents.map((doc) => ({
        kind: "document" as const,
        id: doc.document_id,
        data: doc,
        lastUpdated: doc.timestamp,
        isTerminal: false,
        sourceIssueId: undefined,
      }));
    }
    return issues.map((issue) => ({
      kind: "issue" as const,
      id: issue.issue_id,
      data: issue,
      lastUpdated: issue.timestamp,
      isTerminal: TERMINAL_STATUSES.has(issue.issue.status),
    }));
  }, [isSearching, filterRootId, issues, patchesData, documentsData]);

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
      // Reset filters when switching tabs
      setSelectedIssueStatuses(new Set(ALL_ISSUE_STATUSES));
      setSelectedPatchStatuses(new Set(ALL_PATCH_STATUSES));
      setSelectedLabelId(null);
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

  const fetchNextPage = useCallback(() => {
    if (isSearching) {
      if (hasNextIssues && !isFetchingNextIssues) fetchNextIssues();
      if (hasNextPatches && !isFetchingNextPatches) fetchNextPatches();
      if (hasNextDocuments && !isFetchingNextDocuments) fetchNextDocuments();
      return;
    }
    if (filterRootId === "patches") {
      fetchNextPatches();
    } else if (filterRootId === "documents") {
      fetchNextDocuments();
    } else {
      fetchNextIssues();
    }
  }, [isSearching, filterRootId, fetchNextIssues, fetchNextPatches, fetchNextDocuments, hasNextIssues, hasNextPatches, hasNextDocuments, isFetchingNextIssues, isFetchingNextPatches, isFetchingNextDocuments]);

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) {
      fetchNextPage();
    }
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  return (
    <div className={styles.page}>
      <div className={styles.dashboardRow}>
        <IssueFilterSidebar
          activeFilter={isSearching ? null : filterRootId}
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
          filterBar={
            !isSearching ? (
              <FilterBar
                tabKind={
                  filterRootId === "patches"
                    ? "patches"
                    : filterRootId === "documents"
                      ? "documents"
                      : "issues"
                }
                selectedIssueStatuses={selectedIssueStatuses}
                onIssueStatusesChange={setSelectedIssueStatuses}
                selectedPatchStatuses={selectedPatchStatuses}
                onPatchStatusesChange={setSelectedPatchStatuses}
                selectedLabelId={selectedLabelId}
                onLabelChange={setSelectedLabelId}
              />
            ) : undefined
          }
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
