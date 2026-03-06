import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { useIssues, buildIssueTree } from "../features/issues/useIssues";
import { useAllJobs } from "../features/jobs/useAllJobs";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { IssueFilterSidebar, LABEL_FILTER_PREFIX } from "../features/dashboard/IssueFilterSidebar";
import { HeterogeneousItemList } from "../features/dashboard/HeterogeneousItemList";
import {
  useTransitiveWorkItems,
} from "../features/dashboard/useTransitiveWorkItems";
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

  const roots = useMemo(() => {
    if (!issues) return [];
    const tree = buildIssueTree(issues);
    return tree
      .filter(
        (root) =>
          !root.hardBlocked && root.issue.issue.creator === username,
      )
      .sort(
        (a, b) =>
          new Date(b.issue.creation_time).getTime() -
          new Date(a.issue.creation_time).getTime(),
      );
  }, [issues, username]);

  const assignees = useMemo(() => {
    if (!issues) return [];
    const set = new Set<string>();
    for (const record of issues) {
      if (record.issue.assignee) set.add(record.issue.assignee);
    }
    return Array.from(set).sort();
  }, [issues]);

  const isLabelFilter = filterRootId?.startsWith(LABEL_FILTER_PREFIX) ?? false;
  const hookRootId = filterRootId === "inbox" || isLabelFilter ? null : filterRootId;
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

  const workItems = useMemo(() => {
    if (isLabelFilter) {
      const labelId = filterRootId!.slice(LABEL_FILTER_PREFIX.length);
      return allWorkItems.filter(
        (item) =>
          item.kind === "issue" &&
          item.data.issue.labels?.some((l: { label_id: string }) => l.label_id === labelId),
      );
    }
    if (filterRootId !== "inbox") return allWorkItems;
    if (!inboxLabel) return [];
    return allWorkItems.filter(
      (item) =>
        item.kind === "issue" &&
        item.data.issue.labels?.some((l: { label_id: string }) => l.label_id === inboxLabel.label_id) &&
        (item.data.issue.creator === username || item.data.issue.assignee === username),
    );
  }, [filterRootId, isLabelFilter, allWorkItems, username, inboxLabel]);

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
          roots={roots}
          allIssues={issues ?? []}
          activeFilter={filterRootId}
          onFilterChange={handleFilterChange}
          collapsed={sidebarCollapsed}
          drawerOpen={drawerOpen}
          onDrawerClose={handleDrawerClose}
          jobsByIssue={jobsByIssue ?? new Map()}
          username={username}
          inboxCount={inboxCount}
        />
        <HeterogeneousItemList
          items={workItems}
          jobsByIssue={jobsByIssue ?? new Map()}
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
