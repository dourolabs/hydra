import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { useIssues } from "../features/issues/useIssues";
import { useAllJobs } from "../features/jobs/useAllJobs";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssueFilterSidebar, LABEL_FILTER_PREFIX } from "../features/dashboard/IssueFilterSidebar";
import { HeterogeneousItemList } from "../features/dashboard/HeterogeneousItemList";
import {
  useTransitiveWorkItems,
  findTransitiveChildren,
} from "../features/dashboard/useTransitiveWorkItems";
import { computeIsActiveMap, countNeedsAttentionBadge, type ChildStatus } from "../features/dashboard/computeIssueProgress";
import { TERMINAL_STATUSES } from "../utils/statusMapping";
import { readCollapsed, writeCollapsed } from "../features/dashboard/sidebarStorage";
import { IssueCreateModal } from "../features/dashboard/IssueCreateModal";
import { useInboxLabel } from "../features/labels/useLabels";
import styles from "./DashboardPage.module.css";

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

  const { data: issues, isLoading } = useIssues(searchQuery || undefined);
  const { data: jobsByIssue } = useAllJobs();
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

  const assignees = useMemo(() => {
    if (!issues) return [];
    const set = new Set<string>();
    for (const record of issues) {
      if (record.issue.assignee) set.add(record.issue.assignee);
    }
    return Array.from(set).sort();
  }, [issues]);

  const isLabelFilter = filterRootId?.startsWith(LABEL_FILTER_PREFIX) ?? false;
  const isMyIssuesFilter = filterRootId === "my-issues";
  const hookRootId = filterRootId === "inbox" || isLabelFilter || isMyIssuesFilter ? null : filterRootId;
  const { items: allWorkItems, isLoading: workItemsLoading } =
    useTransitiveWorkItems(hookRootId, issues ?? []);

  const inboxCount = useMemo(() => {
    if (!issues || !inboxLabel) return 0;
    return issues.filter(
      (issue) =>
        !TERMINAL_STATUSES.has(issue.issue.status) &&
        issue.issue.labels?.some((l: { label_id: string }) => l.label_id === inboxLabel.label_id) &&
        (issue.issue.creator === username || issue.issue.assignee === username),
    ).length;
  }, [issues, username, inboxLabel]);

  const isActiveMap = useMemo(() => {
    if (!issues || !jobsByIssue) return new Map<string, boolean>();
    return computeIsActiveMap(issues, jobsByIssue);
  }, [issues, jobsByIssue]);

  const myIssuesCount = useMemo(() => {
    if (!issues || !username) return 0;
    return countNeedsAttentionBadge(issues, (issue) => issue.issue.assignee === username);
  }, [issues, username]);

  const childStatusMap = useMemo(() => {
    const map = new Map<string, ChildStatus[]>();
    if (!issues) return map;
    const childrenByParent = new Map<string, string[]>();
    for (const issue of issues) {
      for (const dep of issue.issue.dependencies) {
        if (dep.type === "child-of") {
          const siblings = childrenByParent.get(dep.issue_id) ?? [];
          siblings.push(issue.issue_id);
          childrenByParent.set(dep.issue_id, siblings);
        }
      }
    }
    const issueById = new Map(issues.map((i) => [i.issue_id, i]));
    for (const [parentId, childIds] of childrenByParent) {
      const statuses: ChildStatus[] = [];
      for (const childId of childIds) {
        const child = issueById.get(childId);
        if (!child) continue;
        statuses.push({
          id: childId,
          status: child.issue.status,
          hasActiveTask: isActiveMap.get(childId) ?? false,
          assignedToUser: !!(username && child.issue.assignee === username),
        });
      }
      if (statuses.length > 0) {
        map.set(parentId, statuses);
      }
    }
    return map;
  }, [issues, isActiveMap, username]);

  const workItems = useMemo(() => {
    // Helper: given matching issue IDs, collect all their transitive descendants
    // and return items that are either matching issues or artifacts from those descendants.
    const filterWithDescendantArtifacts = (
      matchingIssueIds: string[],
    ) => {
      const descendantIds = new Set<string>();
      for (const id of matchingIssueIds) {
        for (const descId of findTransitiveChildren(id, issues ?? [])) {
          descendantIds.add(descId);
        }
      }
      const matchingSet = new Set(matchingIssueIds);
      return allWorkItems.filter((item) => {
        if (item.kind === "issue") {
          return matchingSet.has(item.id);
        }
        // For artifacts, include if their source issue is a descendant
        return descendantIds.has(item.sourceIssueId);
      });
    };

    if (isLabelFilter) {
      const labelId = filterRootId!.slice(LABEL_FILTER_PREFIX.length);
      const matchingIds = allWorkItems
        .filter(
          (item) =>
            item.kind === "issue" &&
            item.data.issue.labels?.some((l: { label_id: string }) => l.label_id === labelId),
        )
        .map((item) => item.id);
      return filterWithDescendantArtifacts(matchingIds);
    }
    if (isMyIssuesFilter) {
      const matchingIds = allWorkItems
        .filter(
          (item) =>
            item.kind === "issue" &&
            item.data.issue.creator === username,
        )
        .map((item) => item.id);
      return filterWithDescendantArtifacts(matchingIds);
    }
    if (filterRootId !== "inbox") return allWorkItems;
    if (!inboxLabel) return [];
    const matchingIds = allWorkItems
      .filter(
        (item) =>
          item.kind === "issue" &&
          item.data.issue.labels?.some((l: { label_id: string }) => l.label_id === inboxLabel.label_id) &&
          (item.data.issue.creator === username || item.data.issue.assignee === username),
      )
      .map((item) => item.id);
    return filterWithDescendantArtifacts(matchingIds);
  }, [filterRootId, isLabelFilter, isMyIssuesFilter, allWorkItems, username, inboxLabel, issues]);

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

  if (isLoading && !issues) {
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
          allIssues={issues ?? []}
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
          items={workItems}
          jobsByIssue={jobsByIssue ?? new Map()}
          childStatusMap={childStatusMap}
          isActiveMap={isActiveMap}
          isLoading={workItemsLoading}
          sidebarCollapsed={sidebarCollapsed}
          onToggleSidebar={handleToggleSidebar}
          onToggleDrawer={handleToggleDrawer}
          filterRootId={filterRootId}
          searchValue={searchValue}
          onSearchChange={handleSearchChange}
          inboxLabelId={filterRootId === "inbox" && inboxLabel ? inboxLabel.label_id : undefined}
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
